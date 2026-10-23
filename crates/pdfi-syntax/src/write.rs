//! Serialization of objects and whole files.

use std::io::{self, Read, Write};

use crate::file::PdfFile;
use crate::lexer::{is_delimiter, is_whitespace};
use crate::object::*;
use crate::xref::XrefEntry;

pub fn write_name(n: &Name, out: &mut Vec<u8>) {
    out.push(b'/');
    for &b in n.as_bytes() {
        if b == b'#' || !(0x21..=0x7e).contains(&b) || is_delimiter(b) || is_whitespace(b) {
            out.extend_from_slice(format!("#{b:02X}").as_bytes());
        } else {
            out.push(b);
        }
    }
}

pub fn write_string(s: &PdfString, out: &mut Vec<u8>) {
    match s.kind {
        StrKind::Hex => {
            out.push(b'<');
            for b in &s.bytes {
                out.extend_from_slice(format!("{b:02X}").as_bytes());
            }
            out.push(b'>');
        }
        StrKind::Literal => {
            out.push(b'(');
            for &b in &s.bytes {
                match b {
                    b'(' | b')' | b'\\' => out.extend_from_slice(&[b'\\', b]),
                    b'\n' => out.extend_from_slice(b"\\n"),
                    b'\r' => out.extend_from_slice(b"\\r"),
                    0x20..=0x7e => out.push(b),
                    _ => out.extend_from_slice(format!("\\{b:03o}").as_bytes()),
                }
            }
            out.push(b')');
        }
    }
}

pub fn format_real(r: f64) -> String {
    if !r.is_finite() {
        return "0".into();
    }
    let s = format!("{r}");
    if s.contains('.') { s } else { format!("{s}.0") }
}

/// Compact serialization (single line).
pub fn write_object(o: &Object, out: &mut Vec<u8>) {
    match o {
        Object::Null => out.extend_from_slice(b"null"),
        Object::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Object::Integer(i) => out.extend_from_slice(i.to_string().as_bytes()),
        Object::Real(r) => out.extend_from_slice(format_real(*r).as_bytes()),
        Object::String(s) => write_string(s, out),
        Object::Name(n) => write_name(n, out),
        Object::Ref(id) => {
            out.extend_from_slice(format!("{} {} R", id.num, id.generation).as_bytes())
        }
        Object::Array(a) => {
            out.push(b'[');
            for (i, n) in a.iter().enumerate() {
                if i > 0 {
                    out.push(b' ');
                }
                write_object(&n.obj, out);
            }
            out.push(b']');
        }
        Object::Dict(d) => write_dict(d, out),
        Object::Stream(s) => write_dict(&s.dict, out),
    }
}

fn write_dict(d: &Dict, out: &mut Vec<u8>) {
    out.extend_from_slice(b"<<");
    for (k, v) in d {
        write_name(k, out);
        out.push(b' ');
        write_object(&v.obj, out);
    }
    out.extend_from_slice(b">>");
}

/// `data` compressed for `/FlateDecode`.
pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    // Writing to a Vec cannot fail.
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// Human-readable multi-line rendering in PDF syntax.
pub fn pretty(o: &Object) -> String {
    let mut s = String::new();
    pretty_into(o, 0, &mut s);
    s
}

fn pretty_into(o: &Object, indent: usize, s: &mut String) {
    let pad = |n: usize| "  ".repeat(n);
    match o {
        Object::Dict(d) | Object::Stream(Stream { dict: d, .. }) => {
            if d.is_empty() {
                s.push_str("<< >>");
                return;
            }
            s.push_str("<<\n");
            for (k, v) in d {
                s.push_str(&pad(indent + 1));
                let mut kb = Vec::new();
                write_name(k, &mut kb);
                s.push_str(&String::from_utf8_lossy(&kb));
                s.push(' ');
                pretty_into(&v.obj, indent + 1, s);
                s.push('\n');
            }
            s.push_str(&pad(indent));
            s.push_str(">>");
        }
        Object::Array(a) => {
            let simple = a.len() <= 12
                && a.iter()
                    .all(|n| !matches!(n.obj, Object::Array(_) | Object::Dict(_)));
            if simple {
                let mut b = Vec::new();
                write_object(o, &mut b);
                s.push_str(&String::from_utf8_lossy(&b));
            } else {
                s.push_str("[\n");
                for n in a {
                    s.push_str(&pad(indent + 1));
                    pretty_into(&n.obj, indent + 1, s);
                    s.push('\n');
                }
                s.push_str(&pad(indent));
                s.push(']');
            }
        }
        other => {
            let mut b = Vec::new();
            write_object(other, &mut b);
            s.push_str(&String::from_utf8_lossy(&b));
        }
    }
}

#[derive(Clone, Debug)]
pub struct RewriteOptions {
    /// Copy the original bytes of objects stored directly in the file
    /// (only possible when the file is not encrypted).
    pub copy_unchanged: bool,
}

impl Default for RewriteOptions {
    fn default() -> Self {
        Self {
            copy_unchanged: true,
        }
    }
}

#[derive(Debug, Default)]
pub struct RewriteReport {
    pub objects: usize,
    pub failed: Vec<(ObjId, String)>,
}

struct Counting<W> {
    inner: W,
    pos: u64,
}

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.pos += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Write a complete new file: every live object as a plain indirect object
/// (object streams expanded), a classic xref table and a single trailer.
/// Encrypted input is written decrypted.
pub fn rewrite(
    file: &PdfFile,
    sink: impl Write,
    opts: &RewriteOptions,
) -> io::Result<RewriteReport> {
    let mut w = Counting {
        inner: io::BufWriter::new(sink),
        pos: 0,
    };
    let mut report = RewriteReport::default();
    let version = file
        .header()
        .version
        .clone()
        .unwrap_or_else(|| "1.7".into());
    w.write_all(format!("%PDF-{version}\n%\u{e2}\u{e3}\u{cf}\u{d3}\n").as_bytes())?;
    let encrypted = file.security().is_some();
    let size = file.xref().size();
    let mut offsets: Vec<Option<(u64, u16)>> = vec![None; size as usize];
    for id in file.xref().live_ids().collect::<Vec<_>>() {
        if Some(id) == file.encrypt_ref() {
            continue;
        }
        let obj = match file.get(id) {
            Ok(o) => o,
            Err(e) => {
                report.failed.push((id, e.to_string()));
                continue;
            }
        };
        if let Object::Stream(s) = &obj.node.obj {
            if s.dict.type_is("XRef") || s.dict.type_is("ObjStm") {
                continue;
            }
        }
        offsets[id.num as usize] = Some((w.pos, id.generation));
        report.objects += 1;
        if let (ObjLocation::File(span), false, true) =
            (obj.location, encrypted, opts.copy_unchanged)
        {
            let mut r = crate::source::RangeReader::new(file.source(), span.start, span.end);
            io::copy(&mut r, &mut w)?;
            w.write_all(b"\n")?;
            continue;
        }
        w.write_all(format!("{} {} obj\n", id.num, id.generation).as_bytes())?;
        let mut buf = Vec::new();
        match &obj.node.obj {
            Object::Stream(s) => {
                let mut data = Vec::new();
                file.decrypted_reader(id, s).read_to_end(&mut data)?;
                let mut dict = s.dict.clone();
                dict.insert(
                    Name::from("Length"),
                    Node::synthetic(Object::Integer(data.len() as i64)),
                );
                // A Crypt filter no longer applies once decrypted.
                write_dict(&dict, &mut buf);
                buf.extend_from_slice(b"\nstream\n");
                w.write_all(&buf)?;
                w.write_all(&data)?;
                w.write_all(b"\nendstream\nendobj\n")?;
            }
            o => {
                write_object(o, &mut buf);
                buf.extend_from_slice(b"\nendobj\n");
                w.write_all(&buf)?;
            }
        }
    }
    let xref_pos = w.pos;
    w.write_all(format!("xref\n0 {size}\n").as_bytes())?;
    for (num, e) in offsets.iter().enumerate() {
        match e {
            Some((off, generation)) => {
                w.write_all(format!("{off:010} {generation:05} n\r\n").as_bytes())?
            }
            None => {
                let generation = if num == 0 {
                    65535
                } else {
                    match file.xref().get(num as u32) {
                        Some(XrefEntry::Free { generation, .. }) => generation,
                        _ => 0,
                    }
                };
                w.write_all(format!("0000000000 {generation:05} f\r\n").as_bytes())?;
            }
        }
    }
    let mut trailer = Dict::new();
    trailer.insert(
        Name::from("Size"),
        Node::synthetic(Object::Integer(i64::from(size))),
    );
    for key in ["Root", "Info", "ID"] {
        if let Some(v) = file.trailer().get(key.as_bytes()) {
            trailer.insert(Name::from(key), v.clone());
        }
    }
    let mut buf = b"trailer\n".to_vec();
    write_dict(&trailer, &mut buf);
    buf.extend_from_slice(format!("\nstartxref\n{xref_pos}\n%%EOF\n").as_bytes());
    w.write_all(&buf)?;
    w.flush()?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_strings() {
        let mut b = Vec::new();
        write_name(&Name::from("A B#"), &mut b);
        assert_eq!(b, b"/A#20B#23");
        let mut b = Vec::new();
        write_string(&PdfString::literal(b"a(b)\\\x01".to_vec()), &mut b);
        assert_eq!(b, b"(a\\(b\\)\\\\\\001)");
        assert_eq!(format_real(1.0), "1.0");
        assert_eq!(format_real(-0.25), "-0.25");
    }
}
