//! Edits: protocol commands turned into `pdfi-edit` commands, and the
//! status of a document's history.

use std::sync::Arc;

use pdfi_content::DocContent;
use pdfi_content::edit as ce;
use pdfi_edit::{self as ed, Command, NodeAt, Root, Saver, Tx};
use pdfi_model::DocModel;
use pdfi_schema::Container;
use pdfi_syntax::file::Edits;
use pdfi_syntax::{DictExt, Name, ObjLocation, Object, PdfFile, Stream, StreamData};

use crate::convert;
use crate::dto::*;
use crate::protocol::{ApiError, ErrorCode};
use crate::session::{Doc, lock};

/// Changed objects whose schema problems an apply reports.
const CHECKED: usize = 50;
const MAX_ISSUES: usize = 100;

fn bad(e: impl std::fmt::Display) -> ApiError {
    ApiError::new(ErrorCode::BadRequest, e.to_string())
}

impl From<ed::EditError> for ApiError {
    fn from(e: ed::EditError) -> Self {
        bad(e)
    }
}

fn node(p: &NodePath) -> NodeAt {
    NodeAt {
        root: match p.root {
            NodeRoot::Trailer => Root::Trailer,
            NodeRoot::Object { id } => Root::Object(id.into()),
        },
        path: p.path.iter().map(|&i| i as usize).collect(),
    }
}

fn value(s: &str) -> Result<pdfi_syntax::Object, ApiError> {
    ed::parse_value(s).map_err(|e| bad(format!("{s:?}: {e}")))
}

fn key(s: &str) -> Result<Name, ApiError> {
    let k = s.strip_prefix('/').unwrap_or(s);
    if k.is_empty() {
        return Err(bad("empty key"));
    }
    Ok(Name::from(k))
}

fn filter(f: StreamFilter) -> ed::StreamFilter {
    match f {
        StreamFilter::None => ed::StreamFilter::None,
        StreamFilter::Flate => ed::StreamFilter::Flate,
    }
}

fn payload(p: &StreamPayload) -> Result<Vec<u8>, ApiError> {
    match p {
        StreamPayload::Text { text } => Ok(text.as_bytes().to_vec()),
        StreamPayload::Base64 { data } => base64(data).ok_or_else(|| bad("bad base64 data")),
    }
}

fn base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            c if c.is_ascii_whitespace() => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// The command, with `others` holding the documents `InsertPages` copies from.
pub(crate) fn command<'a>(
    c: &EditCommand,
    doc: DocId,
    others: &'a [(DocId, Arc<Doc>)],
) -> Result<Command<'a>, ApiError> {
    let pages = |v: &[u32]| v.iter().map(|&i| u64::from(i)).collect();
    Ok(match c {
        EditCommand::SetValue { node: n, value: v } => Command::SetValue {
            at: node(n),
            value: value(v)?,
        },
        EditCommand::SetEntry {
            node: n,
            key: k,
            value: v,
        } => Command::SetEntry {
            at: node(n),
            key: key(k)?,
            value: value(v)?,
        },
        EditCommand::RemoveEntry { node: n, key: k } => Command::RemoveEntry {
            at: node(n),
            key: key(k)?,
        },
        EditCommand::Insert {
            node: n,
            index,
            value: v,
        } => Command::Insert {
            at: node(n),
            index: *index as usize,
            value: value(v)?,
        },
        EditCommand::RemoveChild { node: n, index } => Command::RemoveChild {
            at: node(n),
            index: *index as usize,
        },
        EditCommand::AddObject { value: v, data } => Command::AddObject {
            value: value(v)?,
            data: data.as_ref().map(payload).transpose()?,
        },
        EditCommand::DeleteObject { id } => Command::DeleteObject { id: (*id).into() },
        EditCommand::SetStreamData {
            id,
            data,
            filter: f,
        } => Command::SetStreamData {
            id: (*id).into(),
            data: payload(data)?,
            filter: filter(*f),
        },
        EditCommand::SetStreamFilter { id, filter: f } => Command::SetStreamFilter {
            id: (*id).into(),
            filter: filter(*f),
        },
        EditCommand::DeletePages { pages: p } => Command::DeletePages { pages: pages(p) },
        EditCommand::MovePages { pages: p, to } => Command::MovePages {
            pages: pages(p),
            to: u64::from(*to),
        },
        EditCommand::RotatePages { pages: p, by } => Command::RotatePages {
            pages: pages(p),
            by: i64::from(*by),
        },
        EditCommand::InsertBlankPage { at, width, height } => Command::InsertBlankPage {
            at: u64::from(*at),
            width: *width,
            height: *height,
        },
        EditCommand::InsertPages { at, from, pages: p } => Command::InsertPages {
            at: u64::from(*at),
            from: if *from == doc {
                None
            } else {
                let d = others.iter().find(|(id, _)| id == from).ok_or_else(|| {
                    ApiError::new(ErrorCode::NoSuchDoc, format!("no document {from}"))
                })?;
                Some(&d.1.file)
            },
            pages: pages(p),
        },
        EditCommand::SetInfo {
            key: k,
            value: v,
            sync_xmp,
        } => Command::SetInfo {
            key: key(k)?.as_str_lossy().into_owned(),
            value: v.clone(),
            sync_xmp: *sync_xmp,
        },
        EditCommand::SetXmp { xml } => Command::SetXmp {
            xml: xml.as_bytes().to_vec(),
        },
        EditCommand::EditOps { .. } => {
            return Err(bad("operations are edited by the transaction itself"));
        }
    })
}

/// Apply the commands as one transaction over `file`.
pub(crate) fn transact(
    file: &PdfFile,
    cmds: &[EditCommand],
    doc: DocId,
    others: &[(DocId, Arc<Doc>)],
) -> Result<(Edits, ed::Applied), ApiError> {
    let mut tx = Tx::new(file);
    for (i, c) in cmds.iter().enumerate() {
        let r = match c {
            EditCommand::EditOps { content, edits } => edit_ops(&mut tx, *content, edits),
            c => command(c, doc, others).and_then(|c| Ok(tx.apply(c)?)),
        };
        r.map_err(|e| ApiError::new(e.code, format!("command {}: {}", i + 1, e.message)))?;
    }
    Ok(tx.finish())
}

/// Content stream text (one character per byte) as bytes.
fn latin1(s: &str) -> Result<Vec<u8>, ApiError> {
    s.chars()
        .map(|c| {
            u8::try_from(u32::from(c)).map_err(|_| {
                bad(format!(
                    "{c:?} (U+{:04X}) is not a byte: content streams hold bytes, write it as a <hex> string",
                    u32::from(c)
                ))
            })
        })
        .collect()
}

/// Rewrite the streams of a content for edits of its operations.
fn edit_ops(tx: &mut Tx<'_>, c: ContentRef, edits: &[OpEdit]) -> Result<(), ApiError> {
    let edits = edits
        .iter()
        .map(|e| {
            Ok(match e {
                OpEdit::Replace { from, to, text } => ce::OpEdit::Replace {
                    from: *from as usize,
                    to: *to as usize,
                    text: latin1(text)?,
                },
                OpEdit::Move { from, to, before } => ce::OpEdit::Move {
                    from: *from as usize,
                    to: *to as usize,
                    before: *before as usize,
                },
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    let view = tx.view();
    let (model, content) = (DocModel::new(), DocContent::new());
    let (owner, sources) = content
        .on(model.on(&view))
        .sources(crate::content::target(c))
        .map_err(|e| ApiError::new(ErrorCode::NotFound, e))?;
    let out = ce::splice(&sources, &edits).map_err(bad)?;
    if out.data.iter().all(Option::is_none) {
        return Err(bad("the edit leaves the content as it was"));
    }
    if sources.is_empty() {
        // A page without contents gets a stream.
        let Some(Some(data)) = out.data.into_iter().next() else {
            return Ok(());
        };
        let id = tx.add(Object::Stream(Stream {
            dict: Default::default(),
            data: StreamData::Owned(Vec::new().into()),
        }));
        tx.apply(Command::SetStreamData {
            id,
            data,
            filter: ed::StreamFilter::Flate,
        })?;
        tx.apply(Command::SetEntry {
            at: NodeAt {
                root: Root::Object(owner),
                path: Vec::new(),
            },
            key: Name::from("Contents"),
            value: Object::Ref(id),
        })?;
        return Ok(());
    }
    for (s, data) in sources.iter().zip(out.data) {
        let Some(data) = data else { continue };
        let id =
            s.id.ok_or_else(|| bad("a content stream that is not an object cannot be edited"))?;
        let compressed = matches!(&tx.get(id)?.node.obj, Object::Stream(st) if st.dict.get_key("Filter").is_some());
        tx.apply(Command::SetStreamData {
            id,
            data,
            filter: if compressed {
                ed::StreamFilter::Flate
            } else {
                ed::StreamFilter::None
            },
        })?;
    }
    Ok(())
}

/// Documents an apply copies pages from.
pub(crate) fn sources(cmds: &[EditCommand], doc: DocId) -> Vec<DocId> {
    let mut v: Vec<DocId> = cmds
        .iter()
        .filter_map(|c| match c {
            EditCommand::InsertPages { from, .. } if *from != doc => Some(*from),
            _ => None,
        })
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

pub(crate) fn status(d: &Doc) -> EditStatus {
    let h = lock(&d.history);
    let f = &d.file;
    EditStatus {
        dirty: h.dirty(),
        edited_objects: f.edits().map_or(0, |e| e.objects.len() as u32),
        undo: h.undo_label().map(Into::into),
        redo: h.redo_label().map(Into::into),
        signatures: *d.signatures.get_or_init(|| ed::signature_fields(f) as u32),
        no_incremental: no_incremental(f).map(Into::into),
    }
}

pub(crate) fn no_incremental(f: &PdfFile) -> Option<&'static str> {
    if f.security().is_some() {
        Some("the file is encrypted: it can only be saved decrypted, as a full rewrite")
    } else if f.is_recovered() {
        Some("the file's cross-reference data is broken: save a full rewrite")
    } else {
        None
    }
}

/// Schema problems of the objects an edit changed.
pub(crate) fn issues(d: &Doc, changed: &[u32]) -> Vec<DiagnosticInfo> {
    let ck = d.schema.checker(&d.file);
    let mut out = Vec::new();
    for &num in changed.iter().take(CHECKED) {
        let Ok(o) = d.file.get_num(num) else { continue };
        let (Some((t, _)), Some(c)) = (ck.type_of(o.id), Container::of(&o.node.obj)) else {
            continue;
        };
        let in_file = matches!(o.location, ObjLocation::File(_));
        for i in ck.validate(t, c, 2000).issues.iter() {
            let mut x = convert::issue(i, in_file);
            x.obj = Some(o.id.into());
            out.push(x);
            if out.len() >= MAX_ISSUES {
                return out;
            }
        }
    }
    out
}

pub(crate) fn save_mode(m: SaveMode) -> ed::SaveMode {
    match m {
        SaveMode::Incremental => ed::SaveMode::Incremental,
        SaveMode::Full {
            xref_stream,
            object_streams,
            compress,
            remove_unused,
        } => ed::SaveMode::Full(ed::FullOptions {
            xref_stream,
            object_streams,
            compress,
            remove_unused,
        }),
    }
}

/// A document snapshot as the file a [`Saver`] writes.
pub(crate) struct DocFile(pub Arc<Doc>);

impl std::ops::Deref for DocFile {
    type Target = PdfFile;

    fn deref(&self) -> &PdfFile {
        &self.0.file
    }
}

pub(crate) fn saver(d: Arc<Doc>, mode: SaveMode) -> Result<Saver<DocFile>, ApiError> {
    Ok(Saver::new(DocFile(d), save_mode(mode))?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64() {
        assert_eq!(super::base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(super::base64("aGVsbG8").unwrap(), b"hello");
        assert_eq!(super::base64("AAEC\n/w==").unwrap(), [0, 1, 2, 255]);
        assert!(super::base64("a*b").is_none());
    }
}
