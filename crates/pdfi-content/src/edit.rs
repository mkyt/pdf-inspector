//! Editing a content as an instruction list (docs/DESIGN.md §4.5).
//!
//! Edits address operations by index and bring new ones as text in content
//! stream syntax. A replacement is compared with the operations it replaces
//! (a diff of the instruction lists), so operations left as they were keep
//! their bytes, comments and spacing; only the streams whose bytes change
//! are written again, and each keeps its place in a page's `/Contents`.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use pdfi_syntax::lexer::{is_regular, is_whitespace};
use pdfi_syntax::{Dict, Node, Object, Severity};

use crate::Issues;
use crate::op::{Op, OpOrigin, Operator};
use crate::parse::{MAX_OPS, OpReader, SourceData};

/// Bytes of text one edit may bring in.
pub const MAX_TEXT: usize = 16 << 20;
/// Differences the diff looks for before it treats a whole range as replaced.
const MAX_DIFF: usize = 2000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpEdit {
    /// Replace operations `from..to` with those written in `text`: insert
    /// when `from == to`, remove when the text has no operations. Each edit
    /// sees the result of the ones before it.
    Replace {
        from: usize,
        to: usize,
        text: Vec<u8>,
    },
    /// Move operations `from..to` before operation `before` (which may be
    /// the count, for the end).
    Move {
        from: usize,
        to: usize,
        before: usize,
    },
}

/// What [`splice`] made.
#[derive(Debug, Default, PartialEq)]
pub struct Spliced {
    /// The new decoded data of each stream of the content, `None` where it
    /// is unchanged. A content without streams gets one entry, the data of
    /// a stream to make.
    pub data: Vec<Option<Vec<u8>>>,
    /// Operations in the result.
    pub ops: usize,
}

/// Text of operations, from [`op_text`].
#[derive(Debug, PartialEq)]
pub struct OpText {
    pub text: Vec<u8>,
    /// End of the operations it holds (less than asked when it was too
    /// long).
    pub to: usize,
}

/// Apply `edits` to the content made of `sources`.
pub fn splice(sources: &[SourceData], edits: &[OpEdit]) -> Result<Spliced, String> {
    let idx = Index::new(sources)?;
    let mut seq = Seq::new(idx.ops.len());
    for (k, e) in edits.iter().enumerate() {
        apply(&idx, &mut seq, e).map_err(|m| {
            if edits.len() > 1 {
                format!("edit {}: {m}", k + 1)
            } else {
                m
            }
        })?;
    }
    if seq.len > MAX_OPS {
        return Err(format!("a content holds at most {MAX_OPS} operations"));
    }
    let out = idx.emit(&seq);
    let data = if sources.is_empty() {
        out.into_iter()
            .map(|o| (!o.is_empty()).then_some(o))
            .collect()
    } else {
        out.into_iter()
            .zip(sources)
            .map(|(o, s)| (o != s.data).then_some(o))
            .collect()
    };
    Ok(Spliced { data, ops: seq.len })
}

/// The text of operations `from..to` as written (with the comments and
/// spacing between them), up to about `max` bytes but at least one
/// operation. Operands that began in an earlier stream are included.
pub fn op_text(
    sources: &[SourceData],
    from: usize,
    to: usize,
    max: usize,
) -> Result<OpText, String> {
    let idx = Index::new(sources)?;
    let to = to.min(idx.ops.len());
    let mut text = Vec::new();
    let mut end = from;
    for i in from..to {
        if i > from && text.len() >= max {
            break;
        }
        let o = idx.ops[i];
        let prev = idx.ops[i.saturating_sub(1)];
        if i > from && prev.source == o.source && !idx.carried.contains_key(&i) {
            text.extend_from_slice(idx.slice(o.source, prev.span.end..o.span.end));
        } else {
            if i > from {
                text.push(b'\n');
            }
            text.extend_from_slice(&idx.op_bytes(i));
        }
        end = i + 1;
    }
    Ok(OpText { text, to: end })
}

fn apply(idx: &Index, seq: &mut Seq, e: &OpEdit) -> Result<(), String> {
    match *e {
        OpEdit::Replace { from, to, ref text } => {
            if from > to || to > seq.len {
                return Err(format!(
                    "operations {from}..{to} are not within the {} there are",
                    seq.len
                ));
            }
            if text.len() > MAX_TEXT {
                return Err(format!("text longer than {} MiB", MAX_TEXT >> 20));
            }
            let new = parse_text(text)?;
            let old = seq.take(from, to);
            let parsed: Vec<Option<Cmp>> = old
                .iter()
                .map(|it| match it {
                    Item::Orig(i) => Some(Cmp::parse(&idx.op_bytes(*i))),
                    Item::New(_) => None,
                })
                .collect();
            let cmp = |i: usize| match (&old[i], &parsed[i]) {
                (_, Some(c)) => c,
                (Item::New(n), None) => &n.cmp,
                (Item::Orig(_), None) => unreachable!(),
            };
            let matched = diff(old.len(), new.len(), |i, j| cmp(i).same(&new[j].cmp));
            // A new operation goes to the stream of the one it takes the
            // place of.
            let mut last = None;
            let items = new
                .into_iter()
                .zip(matched)
                .map(|(mut n, m)| match m {
                    Some(i) => {
                        last = Some(i);
                        old[i].clone()
                    }
                    None => {
                        let k = last.map_or(0, |i| i + 1).min(old.len().saturating_sub(1));
                        n.home = old.get(k).and_then(|it| match it {
                            Item::Orig(i) => Some(idx.ops[*i].source as usize),
                            Item::New(o) => o.home,
                        });
                        Item::New(Arc::new(n))
                    }
                })
                .collect();
            seq.insert(from, items);
            Ok(())
        }
        OpEdit::Move { from, to, before } => {
            if from > to || to > seq.len || before > seq.len {
                return Err(format!(
                    "operations {from}..{to} before {before} are not within the {} there are",
                    seq.len
                ));
            }
            if (from + 1..to).contains(&before) {
                return Err("cannot move operations into themselves".into());
            }
            let items = seq.take(from, to);
            let at = if before > from {
                before - (to - from)
            } else {
                before
            };
            seq.insert(at, items);
            Ok(())
        }
    }
}

/// Where the operations of the original content are.
struct Index<'a> {
    sources: &'a [SourceData],
    ops: Vec<OpOrigin>,
    /// Operations whose operands began in an earlier stream, with their
    /// whole text (operands and operator).
    carried: HashMap<usize, Vec<u8>>,
    /// End of the last operation read from each stream (0 when none).
    last_end: Vec<usize>,
}

impl<'a> Index<'a> {
    fn new(sources: &'a [SourceData]) -> Result<Self, String> {
        if let Some((i, e)) = sources
            .iter()
            .enumerate()
            .find_map(|(i, s)| s.error.as_ref().map(|e| (i, e)))
        {
            return Err(format!(
                "content stream [{i}] was not read in full ({e}): it cannot be edited as operations"
            ));
        }
        let mut r = OpReader::new(sources);
        let mut sink = Issues::default();
        let mut ops = Vec::new();
        let mut carried_at = Vec::new();
        while let Some(op) = r.next(&mut sink) {
            if r.carried {
                carried_at.push(ops.len());
            }
            ops.push(op.origin.expect("read operations have an origin"));
        }
        if r.truncated {
            return Err(format!(
                "the content has more than {MAX_OPS} operations: it cannot be edited as operations"
            ));
        }
        let mut last_end = vec![0; sources.len()];
        for o in &ops {
            last_end[o.source as usize] = o.span.end as usize;
        }
        let mut idx = Index {
            sources,
            ops,
            carried: HashMap::new(),
            last_end,
        };
        for c in carried_at {
            let mut text = Vec::new();
            for t in idx.carry_streams(c) {
                text.extend_from_slice(idx.trail(t));
            }
            text.push(b'\n');
            let o = idx.ops[c];
            text.extend_from_slice(idx.slice(o.source, o.span.start..o.span.end));
            idx.carried.insert(c, text);
        }
        Ok(idx)
    }

    fn slice(&self, source: u32, r: Range<u64>) -> &'a [u8] {
        &self.sources[source as usize].data[r.start as usize..r.end as usize]
    }

    /// What follows the last operation of stream `t`.
    fn trail(&self, t: usize) -> &'a [u8] {
        self.sources
            .get(t)
            .map_or(&[], |s| &s.data[self.last_end[t]..])
    }

    /// The streams holding the operands of carried operation `c`.
    fn carry_streams(&self, c: usize) -> Range<usize> {
        let first = if c > 0 {
            self.ops[c - 1].source as usize
        } else {
            0
        };
        first..self.ops[c].source as usize
    }

    /// The operation's text (with its operands, wherever they are).
    fn op_bytes(&self, i: usize) -> Vec<u8> {
        if let Some(t) = self.carried.get(&i) {
            return t.clone();
        }
        let o = self.ops[i];
        self.slice(o.source, o.span.start..o.span.end).to_vec()
    }

    /// The operation with the comments and spacing before it.
    fn with_lead(&self, i: usize) -> &'a [u8] {
        let o = self.ops[i];
        let start = match i.checked_sub(1).map(|p| self.ops[p]) {
            Some(p) if p.source == o.source => p.span.end,
            _ => 0,
        };
        self.slice(o.source, start..o.span.end)
    }

    /// The streams' data for the operations of `seq`.
    fn emit(&self, seq: &Seq) -> Vec<Vec<u8>> {
        let n = self.sources.len().max(1);
        // A carried operation keeps its operands in the earlier stream only
        // while it still follows the operation it followed.
        let mut in_place: HashMap<usize, bool> = self.carried.keys().map(|&c| (c, false)).collect();
        let mut prev: Option<usize> = None;
        let mut first = true;
        seq.for_each(|it| {
            if let Item::Orig(i) = it
                && let Some(p) = in_place.get_mut(i)
            {
                *p = if *i == 0 { first } else { prev == Some(*i - 1) };
            }
            prev = match it {
                Item::Orig(i) => Some(*i),
                Item::New(_) => None,
            };
            first = false;
        });
        let mut suppressed = vec![false; n];
        for (&c, &ok) in &in_place {
            if !ok {
                for t in self.carry_streams(c) {
                    suppressed[t] = true;
                }
            }
        }

        let mut out: Vec<Vec<u8>> = vec![Vec::new(); n];
        let mut cur = 0;
        let close = |out: &mut Vec<Vec<u8>>, t: usize| {
            if !suppressed[t] {
                out[t].extend_from_slice(self.trail(t));
            }
        };
        let mut after_new = false;
        seq.for_each(|it| {
            let (home, bytes, line) = match it {
                Item::Orig(i) => match self.carried.get(i) {
                    Some(t) if !in_place[i] => (Some(self.ops[*i].source as usize), &t[..], true),
                    _ => (
                        Some(self.ops[*i].source as usize),
                        self.with_lead(*i),
                        after_new,
                    ),
                },
                Item::New(op) => (op.home, &op.bytes[..], true),
            };
            if let Some(h) = home {
                while cur < h {
                    close(&mut out, cur);
                    cur += 1;
                }
            }
            if line {
                push_line(&mut out[cur], bytes);
            } else {
                push(&mut out[cur], bytes);
            }
            after_new = matches!(it, Item::New(_));
        });
        while cur < n {
            close(&mut out, cur);
            cur += 1;
        }
        out
    }
}

/// Append `b`, with a space between two tokens that would otherwise run
/// together.
fn push(out: &mut Vec<u8>, b: &[u8]) {
    if out.last().is_some_and(|&c| is_regular(c)) && b.first().is_some_and(|&c| is_regular(c)) {
        out.push(b' ');
    }
    out.extend_from_slice(b);
}

/// Append `b` on a line of its own.
fn push_line(out: &mut Vec<u8>, b: &[u8]) {
    if out.last().is_some_and(|&c| !is_whitespace(c))
        && b.first().is_some_and(|&c| !is_whitespace(c))
    {
        out.push(b'\n');
    }
    out.extend_from_slice(b);
}

/// An operation brought in by an edit.
#[derive(Debug)]
struct NewOp {
    /// Its text with the comments and spacing before it.
    bytes: Vec<u8>,
    cmp: Cmp,
    /// The stream it goes to (`None`: where the operations before it are).
    home: Option<usize>,
}

#[derive(Clone, Debug)]
enum Item {
    Orig(usize),
    New(Arc<NewOp>),
}

#[derive(Debug)]
enum Seg {
    Orig(Range<usize>),
    New(Vec<Arc<NewOp>>),
}

impl Seg {
    fn len(&self) -> usize {
        match self {
            Seg::Orig(r) => r.len(),
            Seg::New(v) => v.len(),
        }
    }
}

/// The edited instruction list: runs of original operations and new ones.
struct Seq {
    segs: Vec<Seg>,
    len: usize,
}

impl Seq {
    fn new(n: usize) -> Self {
        Seq {
            segs: if n > 0 {
                vec![Seg::Orig(0..n)]
            } else {
                Vec::new()
            },
            len: n,
        }
    }

    /// The index of the run starting at operation `at` (split if needed).
    fn split(&mut self, at: usize) -> usize {
        let mut pos = 0;
        for i in 0..self.segs.len() {
            if at == pos {
                return i;
            }
            let l = self.segs[i].len();
            if at < pos + l {
                let k = at - pos;
                let tail = match &mut self.segs[i] {
                    Seg::Orig(r) => {
                        let t = r.start + k..r.end;
                        r.end = r.start + k;
                        Seg::Orig(t)
                    }
                    Seg::New(v) => Seg::New(v.split_off(k)),
                };
                self.segs.insert(i + 1, tail);
                return i + 1;
            }
            pos += l;
        }
        self.segs.len()
    }

    fn take(&mut self, from: usize, to: usize) -> Vec<Item> {
        let i = self.split(from);
        let j = self.split(to);
        let mut items = Vec::with_capacity(to - from);
        for s in self.segs.drain(i..j) {
            match s {
                Seg::Orig(r) => items.extend(r.map(Item::Orig)),
                Seg::New(v) => items.extend(v.into_iter().map(Item::New)),
            }
        }
        self.len -= to - from;
        items
    }

    fn insert(&mut self, at: usize, items: Vec<Item>) {
        let n = items.len();
        let mut segs: Vec<Seg> = Vec::new();
        for it in items {
            match (segs.last_mut(), it) {
                (Some(Seg::Orig(r)), Item::Orig(i)) if r.end == i => r.end += 1,
                (Some(Seg::New(v)), Item::New(op)) => v.push(op),
                (_, Item::Orig(i)) => segs.push(Seg::Orig(i..i + 1)),
                (_, Item::New(op)) => segs.push(Seg::New(vec![op])),
            }
        }
        let i = self.split(at);
        self.segs.splice(i..i, segs);
        self.len += n;
    }

    fn for_each(&self, mut f: impl FnMut(&Item)) {
        for s in &self.segs {
            match s {
                Seg::Orig(r) => r.clone().for_each(|i| f(&Item::Orig(i))),
                Seg::New(v) => v.iter().for_each(|op| f(&Item::New(op.clone()))),
            }
        }
    }
}

/// An operation as compared by the diff: what it means, not how it is
/// written.
#[derive(Debug)]
struct Cmp {
    op: Op,
    /// An inline image's data.
    image: Option<Vec<u8>>,
}

impl Cmp {
    fn of(op: Op, data: &[u8]) -> Self {
        let image = op.inline_image.as_ref().map(|img| {
            data.get(img.data.start as usize..img.data.end as usize)
                .unwrap_or_default()
                .to_vec()
        });
        Cmp { op, image }
    }

    /// The single operation written in `b`.
    fn parse(b: &[u8]) -> Self {
        let src = [SourceData {
            id: None,
            data: b.to_vec(),
            error: None,
        }];
        let op = OpReader::new(&src)
            .next(&mut Issues::default())
            .unwrap_or(Op {
                operator: Operator::Unknown(Box::default()),
                operands: Vec::new(),
                origin: None,
                inline_image: None,
            });
        Cmp::of(op, &src[0].data)
    }

    fn same(&self, o: &Cmp) -> bool {
        self.op.operator == o.op.operator
            && same_nodes(&self.op.operands, &o.op.operands)
            && self.image == o.image
            && match (&self.op.inline_image, &o.op.inline_image) {
                (Some(a), Some(b)) => same_dicts(&a.dict, &b.dict),
                (None, None) => true,
                _ => false,
            }
    }
}

fn same_nodes(a: &[Node], b: &[Node]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same(&x.obj, &y.obj))
}

fn same_dicts(a: &Dict, b: &Dict) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|((ka, va), (kb, vb))| ka == kb && same(&va.obj, &vb.obj))
}

/// Equal values, wherever they were read from.
fn same(a: &Object, b: &Object) -> bool {
    match (a, b) {
        (Object::Real(x), Object::Real(y)) => x.to_bits() == y.to_bits(),
        (Object::Array(x), Object::Array(y)) => same_nodes(x, y),
        (Object::Dict(x), Object::Dict(y)) => same_dicts(x, y),
        _ => a == b,
    }
}

/// The operations written in `text`; syntax errors and operands without an
/// operator are refused.
fn parse_text(text: &[u8]) -> Result<Vec<NewOp>, String> {
    let src = [SourceData {
        id: None,
        data: text.to_vec(),
        error: None,
    }];
    let mut issues = Issues::default();
    let mut r = OpReader::new(&src);
    let mut ops = Vec::new();
    while let Some(op) = r.next(&mut issues) {
        ops.push(op);
    }
    if let Some(i) = issues
        .into_vec()
        .into_iter()
        .find(|i| i.severity == Severity::Error || i.code == "Operands")
    {
        return Err(match i.at {
            Some(at) => format!("{}: {}", position(text, at.span.start as usize), i.message),
            None => i.message,
        });
    }
    let mut prev = 0;
    let mut out: Vec<NewOp> = Vec::with_capacity(ops.len());
    for op in ops {
        let end = op.origin.map_or(prev, |o| o.span.end as usize);
        let bytes = text[prev..end].to_vec();
        prev = end;
        out.push(NewOp {
            bytes,
            cmp: Cmp::of(op, text),
            home: None,
        });
    }
    // Comments after the last operation stay with it.
    let tail = &text[prev..];
    if let Some(last) = out.last_mut()
        && tail.iter().any(|&b| !is_whitespace(b))
    {
        last.bytes.extend_from_slice(tail);
        if !tail.ends_with(b"\n") && !tail.ends_with(b"\r") {
            last.bytes.push(b'\n');
        }
    }
    Ok(out)
}

/// `line L, column C` of a byte offset (both from 1).
fn position(text: &[u8], at: usize) -> String {
    let before = &text[..at.min(text.len())];
    let line = before.iter().filter(|&&b| b == b'\n').count() + 1;
    let col = before.len()
        - before
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |p| p + 1)
        + 1;
    format!("line {line}, column {col}")
}

/// For each of `m` new items, the old one (of `n`) it matches: a longest
/// common subsequence, or none in the middle when the lists differ in more
/// than [`MAX_DIFF`] places.
fn diff(n: usize, m: usize, eq: impl Fn(usize, usize) -> bool) -> Vec<Option<usize>> {
    let mut out = vec![None; m];
    let mut pre = 0;
    while pre < n && pre < m && eq(pre, pre) {
        out[pre] = Some(pre);
        pre += 1;
    }
    let mut suf = 0;
    while suf < n - pre && suf < m - pre && eq(n - 1 - suf, m - 1 - suf) {
        out[m - 1 - suf] = Some(n - 1 - suf);
        suf += 1;
    }
    let (a, b) = (n - pre - suf, m - pre - suf);
    if a > 0 && b > 0 {
        for (i, j) in myers(a, b, |i, j| eq(pre + i, pre + j)) {
            out[pre + j] = Some(pre + i);
        }
    }
    out
}

/// Matching pairs of Myers' shortest edit script (none past [`MAX_DIFF`]).
fn myers(n: usize, m: usize, eq: impl Fn(usize, usize) -> bool) -> Vec<(usize, usize)> {
    let (n, m) = (n as isize, m as isize);
    let max = (n + m).min(MAX_DIFF as isize);
    let off = max + 1;
    let mut v = vec![0isize; 2 * max as usize + 3];
    // Before step d: v[-d-1 ..= d+1].
    let mut trace: Vec<Vec<isize>> = Vec::new();
    for d in 0..=max {
        trace.push(v[(off - d - 1) as usize..=(off + d + 1) as usize].to_vec());
        let mut k = -d;
        while k <= d {
            let mut x =
                if k == -d || (k != d && v[(off + k - 1) as usize] < v[(off + k + 1) as usize]) {
                    v[(off + k + 1) as usize]
                } else {
                    v[(off + k - 1) as usize] + 1
                };
            let mut y = x - k;
            while x < n && y < m && eq(x as usize, y as usize) {
                x += 1;
                y += 1;
            }
            v[(off + k) as usize] = x;
            if x >= n && y >= m {
                return backtrack(&trace, n, m);
            }
            k += 2;
        }
    }
    Vec::new()
}

fn backtrack(trace: &[Vec<isize>], n: isize, m: isize) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (0..trace.len() as isize).rev() {
        let v = &trace[d as usize];
        let at = |k: isize| v[(k + d + 1) as usize];
        let k = x - y;
        let pk = if k == -d || (k != d && at(k - 1) < at(k + 1)) {
            k + 1
        } else {
            k - 1
        };
        let px = at(pk);
        let py = px - pk;
        while x > px && y > py {
            pairs.push(((x - 1) as usize, (y - 1) as usize));
            x -= 1;
            y -= 1;
        }
        x = px;
        y = py;
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(parts: &[&str]) -> Vec<SourceData> {
        parts
            .iter()
            .map(|p| SourceData {
                id: None,
                data: p.as_bytes().to_vec(),
                error: None,
            })
            .collect()
    }

    fn run(parts: &[&str], edits: &[OpEdit]) -> Vec<Option<String>> {
        splice(&src(parts), edits)
            .unwrap()
            .data
            .into_iter()
            .map(|d| d.map(|d| String::from_utf8(d).unwrap()))
            .collect()
    }

    fn replace(from: usize, to: usize, text: &str) -> OpEdit {
        OpEdit::Replace {
            from,
            to,
            text: text.as_bytes().to_vec(),
        }
    }

    #[test]
    fn unchanged_ops_keep_their_bytes() {
        let page = "q % keep\n1 0 0 1 10 20 cm\n0 0 m 5 5 l S\nQ\n";
        // The same operations written differently: nothing changes.
        let r = run(
            &[page],
            &[replace(0, 6, "q 1 0 0 1 10 20 cm 0 0 m 5 5 l S Q")],
        );
        assert_eq!(r, [None]);
        // One changed operand: only that operation is written anew.
        let r = run(
            &[page],
            &[replace(0, 6, "q 1 0 0 1 10 20 cm 0 0 m 7 7 l S Q")],
        );
        assert_eq!(
            r,
            [Some(
                "q % keep\n1 0 0 1 10 20 cm\n0 0 m 7 7 l S\nQ\n".into()
            )]
        );
    }

    #[test]
    fn insert_remove_and_move() {
        let page = "q\n0 g\n0 0 10 10 re f\nQ\n";
        assert_eq!(
            run(&[page], &[replace(1, 1, "1 0 0 RG")]),
            [Some("q\n1 0 0 RG\n0 g\n0 0 10 10 re f\nQ\n".into())]
        );
        assert_eq!(
            run(&[page], &[replace(1, 2, "")]),
            [Some("q\n0 0 10 10 re f\nQ\n".into())]
        );
        assert_eq!(
            run(
                &[page],
                &[OpEdit::Move {
                    from: 2,
                    to: 4,
                    before: 1
                }]
            ),
            [Some("q\n0 0 10 10 re f\n0 g\nQ\n".into())]
        );
        // Edits see the ones before them.
        assert_eq!(
            run(&[page], &[replace(0, 0, "BT"), replace(6, 6, "ET")]),
            [Some("BT\nq\n0 g\n0 0 10 10 re f\nQ\nET\n".into())]
        );
    }

    #[test]
    fn streams_keep_their_operations() {
        let parts = ["q 0 g\n", "0 0 1 1 re f\n", "Q\n"];
        let r = run(&parts, &[replace(2, 3, "0 0 2 2 re")]);
        assert_eq!(r, [None, Some("0 0 2 2 re f\n".into()), None]);
        // Moving an operation into another stream rewrites both.
        let r = run(
            &parts,
            &[OpEdit::Move {
                from: 1,
                to: 2,
                before: 4,
            }],
        );
        assert_eq!(
            r,
            [Some("q\n".into()), Some("0 0 1 1 re f 0 g\n".into()), None]
        );
    }

    #[test]
    fn operands_in_an_earlier_stream() {
        let parts = ["q 1 0 0", " RG Q"];
        // Left alone, they stay split.
        assert_eq!(
            run(&parts, &[replace(2, 3, "")]),
            [None, Some(" RG".into())]
        );
        // Edited, the operands go with the operator.
        let r = run(&parts, &[replace(1, 2, "0 1 0 RG")]);
        assert_eq!(r, [Some("q".into()), Some("0 1 0 RG Q".into())]);
        let t = op_text(&src(&parts), 0, 3, 1000).unwrap();
        assert_eq!(t.text, b"q\n 1 0 0\nRG Q");
    }

    #[test]
    fn text_with_errors_is_refused() {
        let e = splice(&src(&["q Q"]), &[replace(1, 1, "0 0 m\n(open")]).unwrap_err();
        assert!(e.starts_with("line 2, column 1"), "{e}");
        let e = splice(&src(&["q Q"]), &[replace(1, 1, "1 2")]).unwrap_err();
        assert!(e.contains("without an operator"), "{e}");
        assert!(splice(&src(&["q Q"]), &[replace(1, 3, "")]).is_err());
    }

    #[test]
    fn empty_content_gets_a_stream() {
        let s = splice(&[], &[replace(0, 0, "0 g 0 0 1 1 re f")]).unwrap();
        assert_eq!(s.data, [Some(b"0 g 0 0 1 1 re f".to_vec())]);
        assert_eq!(s.ops, 3);
    }

    #[test]
    fn inline_images_compare_by_data() {
        let page = "BI /W 2 /H 1 /CS /G /BPC 8 ID ab EI Q";
        assert_eq!(
            run(
                &[page],
                &[replace(0, 1, "BI /W 2 /H 1 /CS /G /BPC 8 ID ab EI")]
            ),
            [None]
        );
        assert_eq!(
            run(
                &[page],
                &[replace(0, 1, "BI /W 2 /H 1 /CS /G /BPC 8 ID ac EI")]
            ),
            [Some("BI /W 2 /H 1 /CS /G /BPC 8 ID ac EI Q".into())]
        );
    }

    #[test]
    fn diff_finds_common_operations() {
        let a = [1, 2, 3, 4, 5, 6];
        let b = [1, 9, 3, 4, 6, 7];
        let m = diff(a.len(), b.len(), |i, j| a[i] == b[j]);
        assert_eq!(m, [Some(0), None, Some(2), Some(3), Some(5), None]);
        let m = diff(3, 0, |_, _| true);
        assert!(m.is_empty());
    }
}
