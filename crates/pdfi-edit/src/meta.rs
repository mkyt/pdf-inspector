//! The Info dictionary and the catalog's XMP metadata.

use pdfi_model::text::{Date, pdf_date, xmp_date};
use pdfi_model::xmp::{DC, PDF, RDF, XMP};
use pdfi_syntax::{Dict, DictExt, Name, Node, ObjId, Object, Stream, StreamData};
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::name::{QName, ResolveResult};
use quick_xml::reader::NsReader;
use quick_xml::writer::Writer;

use crate::value::text_string;
use crate::{EditError, Result, Tx, err};

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    /// `rdf:Alt` with an `x-default` item.
    Alt,
    /// `rdf:Seq`.
    Seq,
    Simple,
    Date,
}

/// Info keys and the XMP properties they correspond to.
const XMP_OF: [(&str, &str, &str, Shape); 8] = [
    ("Title", DC, "title", Shape::Alt),
    ("Author", DC, "creator", Shape::Seq),
    ("Subject", DC, "description", Shape::Alt),
    ("Keywords", PDF, "Keywords", Shape::Simple),
    ("Creator", XMP, "CreatorTool", Shape::Simple),
    ("Producer", PDF, "Producer", Shape::Simple),
    ("CreationDate", XMP, "CreateDate", Shape::Date),
    ("ModDate", XMP, "ModifyDate", Shape::Date),
];

fn prefix_of(ns: &str) -> &'static str {
    match ns {
        DC => "dc",
        XMP => "xmp",
        PDF => "pdf",
        _ => "ns",
    }
}

/// `D:YYYYMMDDHHmmSS+HH'mm'` for a date.
pub fn pdf_date_text(d: &Date) -> String {
    let mut s = format!(
        "D:{:04}{:02}{:02}{:02}{:02}{:02}",
        d.year, d.month, d.day, d.hour, d.minute, d.second
    );
    if let Some(o) = d.offset {
        if o == 0 {
            s.push('Z');
        } else {
            let sign = if o < 0 { '-' } else { '+' };
            let o = o.unsigned_abs();
            s += &format!("{sign}{:02}'{:02}'", o / 60, o % 60);
        }
    }
    s
}

fn xmp_date_text(d: &Date) -> String {
    let mut s = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        d.year, d.month, d.day, d.hour, d.minute, d.second
    );
    match d.offset {
        Some(0) => s.push('Z'),
        Some(o) => {
            let sign = if o < 0 { '-' } else { '+' };
            let o = o.unsigned_abs();
            s += &format!("{sign}{:02}:{:02}", o / 60, o % 60);
        }
        None => {}
    }
    s
}

/// A date typed as a PDF date or ISO 8601.
fn parse_date(text: &str) -> Option<Date> {
    pdf_date(text.trim().as_bytes())
        .filter(|_| text.trim().starts_with("D:"))
        .or_else(|| xmp_date(text))
}

/// The value an Info entry gets for `text`.
pub fn info_text(key: &str, text: &str) -> Result<Object> {
    Ok(match key {
        "CreationDate" | "ModDate" => {
            let d = parse_date(text).ok_or_else(|| EditError(format!("{text:?} is not a date")))?;
            Object::String(text_string(&pdf_date_text(&d)))
        }
        "Trapped" => Object::Name(Name::from(text.trim().trim_start_matches('/'))),
        _ => Object::String(text_string(text)),
    })
}

pub(crate) fn set_info(tx: &mut Tx, key: &str, value: Option<&str>, sync: bool) -> Result<()> {
    if key.is_empty() {
        return err("empty key");
    }
    let new = value.map(|v| info_text(key, v)).transpose()?;
    let mut trailer = tx.trailer().clone();
    let (id, mut info) = match trailer.get_key("Info") {
        Some(Object::Ref(r)) => match tx.resolve(&Object::Ref(*r)) {
            Object::Dict(d) => (Some(*r), d),
            _ => (Some(*r), Dict::new()),
        },
        Some(Object::Dict(d)) => (None, d.clone()),
        _ => (None, Dict::new()),
    };
    match &new {
        Some(v) => {
            info.insert(Name::from(key), Node::synthetic(v.clone()));
        }
        None => {
            info.shift_remove(key.as_bytes());
        }
    }
    match id {
        Some(id) => tx.put(id, Object::Dict(info)),
        None => {
            let id = tx.add(Object::Dict(info));
            trailer.insert(Name::from("Info"), Node::synthetic(Object::Ref(id)));
            tx.set_trailer(trailer);
        }
    }
    if !sync {
        return Ok(());
    }
    let Some(&(_, ns, local, shape)) = XMP_OF.iter().find(|(k, ..)| *k == key) else {
        tx.note(format!("/Info /{key} has no XMP counterpart"));
        return Ok(());
    };
    let Some((mid, xml)) = metadata(tx)? else {
        return Ok(());
    };
    let value = match (value, shape) {
        (Some(v), Shape::Date) => parse_date(v).map(|d| xmp_date_text(&d)),
        (v, _) => v.map(str::to_owned),
    };
    let xml = xmp_set(&xml, ns, local, value.as_deref(), shape)?;
    write_metadata(tx, mid, xml)
}

/// The catalog's metadata stream and its decoded data.
fn metadata(tx: &Tx) -> Result<Option<(ObjId, Vec<u8>)>> {
    let (_, cat) = tx.catalog()?;
    let Some(id) = cat.get_key("Metadata").and_then(Object::as_ref_id) else {
        return Ok(None);
    };
    let o = tx.get(id)?;
    let Object::Stream(s) = &o.node.obj else {
        return err("/Metadata is not a stream");
    };
    let view = tx.view();
    let d = view.decode_stream(id, s, Some(64 << 20));
    if let Some(e) = d.errors.first() {
        return err(format!("cannot decode the metadata: {e}"));
    }
    Ok(Some((id, d.data)))
}

/// Metadata streams stay unfiltered so tools that do not read PDF find them.
fn write_metadata(tx: &mut Tx, id: ObjId, xml: Vec<u8>) -> Result<()> {
    let mut dict = match &tx.get(id)?.node.obj {
        Object::Stream(s) => s.dict.clone(),
        _ => Dict::new(),
    };
    for k in ["Filter", "DecodeParms", "DL"] {
        dict.shift_remove(k.as_bytes());
    }
    dict.insert(
        Name::from("Type"),
        Node::synthetic(Object::Name(Name::from("Metadata"))),
    );
    dict.insert(
        Name::from("Subtype"),
        Node::synthetic(Object::Name(Name::from("XML"))),
    );
    dict.insert(
        Name::from("Length"),
        Node::synthetic(Object::Integer(xml.len() as i64)),
    );
    tx.put(
        id,
        Object::Stream(Stream {
            dict,
            data: StreamData::Owned(xml.into()),
        }),
    );
    Ok(())
}

pub(crate) fn set_xmp(tx: &mut Tx, xml: Vec<u8>) -> Result<()> {
    check_xml(&xml)?;
    if let Some((id, _)) = metadata(tx).ok().flatten() {
        return write_metadata(tx, id, xml);
    }
    let (cid, mut cat) = tx.catalog()?;
    let id = tx.add(Object::Null);
    write_metadata(tx, id, xml)?;
    cat.insert(Name::from("Metadata"), Node::synthetic(Object::Ref(id)));
    tx.put(cid, Object::Dict(cat));
    Ok(())
}

fn check_xml(xml: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(xml).map_err(|_| EditError("XMP must be UTF-8".into()))?;
    let mut r = NsReader::from_str(text);
    loop {
        match r.read_event() {
            Ok(Event::Eof) => return Ok(()),
            Ok(_) => {}
            Err(e) => return err(format!("XMP is not well-formed: {e}")),
        }
    }
}

fn xml_err(e: impl std::fmt::Display) -> EditError {
    EditError(format!("cannot rewrite the XMP: {e}"))
}

fn is(r: &ResolveResult, ns: &str) -> bool {
    matches!(r, ResolveResult::Bound(n) if n.as_ref() == ns.as_bytes())
}

/// Write the property's value: `<rdf:Alt>…`, `<rdf:Seq>…` or text.
fn write_value(w: &mut Writer<Vec<u8>>, rdf: &str, value: &str, shape: Shape) -> Result<()> {
    let li = format!("{rdf}:li");
    let wrap = match shape {
        Shape::Alt => Some(format!("{rdf}:Alt")),
        Shape::Seq => Some(format!("{rdf}:Seq")),
        _ => None,
    };
    let mut ev = |e: Event| w.write_event(e).map_err(xml_err);
    if let Some(c) = &wrap {
        ev(Event::Start(BytesStart::new(c.as_str())))?;
        let mut s = BytesStart::new(li.as_str());
        if shape == Shape::Alt {
            s.push_attribute(("xml:lang", "x-default"));
        }
        ev(Event::Start(s))?;
    }
    ev(Event::Text(BytesText::new(value)))?;
    if let Some(c) = &wrap {
        ev(Event::End(BytesEnd::new(li.as_str())))?;
        ev(Event::End(BytesEnd::new(c.as_str())))?;
    }
    Ok(())
}

/// Set (or with `None` remove) one property of the first `rdf:Description`
/// that has it, as an element or an attribute; add it as an element of the
/// first description if no description has it.
fn xmp_set(
    xml: &[u8],
    ns: &str,
    local: &str,
    value: Option<&str>,
    shape: Shape,
) -> Result<Vec<u8>> {
    let bom = xml.starts_with(b"\xef\xbb\xbf");
    let text = std::str::from_utf8(&xml[if bom { 3 } else { 0 }..])
        .map_err(|_| EditError("the XMP is not UTF-8".into()))?;
    let mut r = NsReader::from_str(text);
    let mut w = Writer::new(Vec::new());
    let mut depth = 0usize;
    // Depth of the description we are in, and its rdf prefix.
    let mut desc: Option<usize> = None;
    let mut rdf = String::from("rdf");
    let mut done = false;
    // Skipping the old value of the property: its depth.
    let mut skip: Option<usize> = None;
    let mut first_desc_end = true;
    let prop = |r: &NsReader<&[u8]>, name: QName| {
        let (n, l) = r.resolver().resolve_element(name);
        is(&n, ns) && l.as_ref() == local.as_bytes()
    };
    loop {
        let ev = r.read_event().map_err(xml_err)?;
        if let Some(d) = skip {
            match &ev {
                Event::Start(_) => depth += 1,
                Event::End(e) => {
                    if depth == d {
                        skip = None;
                        if value.is_some() {
                            w.write_event(Event::End(e.clone())).map_err(xml_err)?;
                        }
                    }
                    depth -= 1;
                }
                Event::Eof => return Err(xml_err("unexpected end")),
                _ => {}
            }
            continue;
        }
        match ev {
            Event::Eof => break,
            Event::Start(e) => {
                depth += 1;
                let (n, l) = r.resolver().resolve_element(e.name());
                if desc.is_none() && is(&n, RDF) && l.as_ref() == b"Description" {
                    desc = Some(depth);
                    if let Some(p) = e.name().prefix() {
                        rdf = String::from_utf8_lossy(p.as_ref()).into_owned();
                    }
                    let (e, hit) = attr_set(&r, e, ns, local, value, shape)?;
                    done |= hit;
                    w.write_event(Event::Start(e)).map_err(xml_err)?;
                    continue;
                }
                if !done && desc == Some(depth - 1) && prop(&r, e.name()) {
                    done = true;
                    skip = Some(depth);
                    if let Some(v) = value {
                        w.write_event(Event::Start(e)).map_err(xml_err)?;
                        write_value(&mut w, &rdf, v, shape)?;
                    }
                    continue;
                }
                w.write_event(Event::Start(e)).map_err(xml_err)?;
            }
            Event::Empty(e) => {
                let (n, l) = r.resolver().resolve_element(e.name());
                if desc.is_none() && is(&n, RDF) && l.as_ref() == b"Description" {
                    if let Some(p) = e.name().prefix() {
                        rdf = String::from_utf8_lossy(p.as_ref()).into_owned();
                    }
                    let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                    let (e, hit) = attr_set(&r, e, ns, local, value, shape)?;
                    done |= hit;
                    if done || value.is_none() || !first_desc_end {
                        w.write_event(Event::Empty(e)).map_err(xml_err)?;
                    } else {
                        // Open it to add the property.
                        w.write_event(Event::Start(e)).map_err(xml_err)?;
                        add_property(&mut w, &rdf, ns, local, value, shape)?;
                        w.write_event(Event::End(BytesEnd::new(name)))
                            .map_err(xml_err)?;
                        done = true;
                    }
                    first_desc_end = false;
                    continue;
                }
                if !done && desc == Some(depth) && prop(&r, e.name()) {
                    done = true;
                    if let Some(v) = value {
                        let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                        w.write_event(Event::Start(e)).map_err(xml_err)?;
                        write_value(&mut w, &rdf, v, shape)?;
                        w.write_event(Event::End(BytesEnd::new(name)))
                            .map_err(xml_err)?;
                    }
                    continue;
                }
                w.write_event(Event::Empty(e)).map_err(xml_err)?;
            }
            Event::End(e) => {
                if desc == Some(depth) {
                    if !done && first_desc_end && value.is_some() {
                        add_property(&mut w, &rdf, ns, local, value, shape)?;
                        done = true;
                    }
                    first_desc_end = false;
                    desc = None;
                }
                depth = depth.saturating_sub(1);
                w.write_event(Event::End(e)).map_err(xml_err)?;
            }
            other => w.write_event(other).map_err(xml_err)?,
        }
    }
    if !done && value.is_some() {
        return err("the XMP has no rdf:Description to add the property to");
    }
    let mut out = if bom {
        b"\xef\xbb\xbf".to_vec()
    } else {
        Vec::new()
    };
    out.extend(w.into_inner());
    Ok(out)
}

fn add_property(
    w: &mut Writer<Vec<u8>>,
    rdf: &str,
    ns: &str,
    local: &str,
    value: Option<&str>,
    shape: Shape,
) -> Result<()> {
    let Some(v) = value else { return Ok(()) };
    let p = prefix_of(ns);
    let name = format!("{p}:{local}");
    let mut s = BytesStart::new(name.as_str());
    s.push_attribute((format!("xmlns:{p}").as_str(), ns));
    w.write_event(Event::Start(s)).map_err(xml_err)?;
    write_value(w, rdf, v, shape)?;
    w.write_event(Event::End(BytesEnd::new(name.as_str())))
        .map_err(xml_err)
}

/// The description start tag with the property as an attribute replaced
/// (text shapes) or removed (removal, or a shape attributes cannot hold).
fn attr_set<'e>(
    r: &NsReader<&[u8]>,
    e: BytesStart<'e>,
    ns: &str,
    local: &str,
    value: Option<&str>,
    shape: Shape,
) -> Result<(BytesStart<'e>, bool)> {
    let mut hit = false;
    let mut attrs = Vec::new();
    for a in e.attributes().with_checks(false) {
        let a = a.map_err(xml_err)?;
        let (n, l) = r.resolver().resolve_attribute(a.key);
        if !hit && is(&n, ns) && l.as_ref() == local.as_bytes() {
            hit = true;
            if let (Some(v), Shape::Simple | Shape::Date) = (value, shape) {
                attrs.push((
                    a.key.as_ref().to_vec(),
                    quick_xml::escape::escape(v).into_owned().into_bytes(),
                ));
            }
            continue;
        }
        attrs.push((a.key.as_ref().to_vec(), a.value.into_owned()));
    }
    if !hit {
        return Ok((e, false));
    }
    let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
    let mut s = BytesStart::new(name);
    for (k, v) in &attrs {
        s.push_attribute(quick_xml::events::attributes::Attribute {
            key: QName(k),
            value: v.as_slice().into(),
        });
    }
    // An Alt or Seq value cannot be an attribute: it was removed, and the
    // caller adds it as an element when it reaches the end.
    let kept = value.is_none() || matches!(shape, Shape::Simple | Shape::Date);
    Ok((s, kept))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACKET: &str = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:pdf="http://ns.adobe.com/pdf/1.3/" pdf:Producer="Old">
<dc:title xmlns:dc="http://purl.org/dc/elements/1.1/"><rdf:Alt><rdf:li xml:lang="x-default">Old title</rdf:li></rdf:Alt></dc:title>
</rdf:Description></rdf:RDF></x:xmpmeta>
<?xpacket end="w"?>"#;

    fn get(xml: &[u8], ns: &str, local: &str) -> Option<String> {
        pdfi_model::xmp::parse(xml, 1000)
            .property(ns, local)
            .map(str::to_owned)
    }

    #[test]
    fn element_and_attribute() {
        let x = xmp_set(
            PACKET.as_bytes(),
            DC,
            "title",
            Some("New & <title>"),
            Shape::Alt,
        )
        .unwrap();
        assert_eq!(get(&x, DC, "title").as_deref(), Some("New & <title>"));
        let x = xmp_set(&x, PDF, "Producer", Some("pdfi"), Shape::Simple).unwrap();
        assert_eq!(get(&x, PDF, "Producer").as_deref(), Some("pdfi"));
        let x = xmp_set(
            &x,
            XMP,
            "ModifyDate",
            Some("2026-10-24T10:00:00Z"),
            Shape::Date,
        )
        .unwrap();
        assert_eq!(
            get(&x, XMP, "ModifyDate").as_deref(),
            Some("2026-10-24T10:00:00Z")
        );
        let x = xmp_set(&x, DC, "creator", Some("Me"), Shape::Seq).unwrap();
        assert_eq!(get(&x, DC, "creator").as_deref(), Some("Me"));
        let x = xmp_set(&x, DC, "title", None, Shape::Alt).unwrap();
        assert_eq!(get(&x, DC, "title"), None);
        let x = xmp_set(&x, PDF, "Producer", None, Shape::Simple).unwrap();
        assert_eq!(get(&x, PDF, "Producer"), None);
        assert!(String::from_utf8(x).unwrap().contains("xpacket end"));
    }

    #[test]
    fn dates() {
        let d = parse_date("2026-10-24T09:30:00+09:00").unwrap();
        assert_eq!(pdf_date_text(&d), "D:20261024093000+09'00'");
        let d = parse_date("D:20261024093000Z").unwrap();
        assert_eq!(xmp_date_text(&d), "2026-10-24T09:30:00Z");
        assert!(info_text("ModDate", "yesterday").is_err());
    }
}
