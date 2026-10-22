//! XMP metadata packets: the XML as a flat tree, and the RDF properties.

use quick_xml::events::Event;
use quick_xml::events::attributes::Attribute;
use quick_xml::name::{QName, ResolveResult};
use quick_xml::reader::NsReader;

pub const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
pub const DC: &str = "http://purl.org/dc/elements/1.1/";
pub const XMP: &str = "http://ns.adobe.com/xap/1.0/";
pub const PDF: &str = "http://ns.adobe.com/pdf/1.3/";

/// Element nesting we follow.
const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Element,
    Attribute,
}

/// One element or attribute; children follow their parent with a greater
/// `depth` (the tree is flattened so that it can be paged).
#[derive(Clone, Debug, PartialEq)]
pub struct XmpNode {
    pub depth: u32,
    pub kind: NodeKind,
    /// Qualified name as written (`dc:title`).
    pub name: String,
    pub namespace: Option<String>,
    /// Text content of an element without child elements, or the value of an
    /// attribute.
    pub value: Option<String>,
}

/// A property of an `rdf:Description`, with arrays reduced to text.
#[derive(Clone, Debug, PartialEq)]
pub struct XmpProperty {
    pub name: String,
    pub namespace: String,
    pub local: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Xmp {
    pub nodes: Vec<XmpNode>,
    /// More nodes than were kept.
    pub truncated: bool,
    pub properties: Vec<XmpProperty>,
    pub error: Option<String>,
}

impl Xmp {
    pub fn property(&self, ns: &str, local: &str) -> Option<&str> {
        self.properties
            .iter()
            .find(|p| p.namespace == ns && p.local == local)
            .map(|p| p.value.as_str())
    }
}

struct El {
    name: String,
    ns: Option<String>,
    local: String,
    attrs: Vec<At>,
    children: Vec<El>,
    text: String,
}

struct At {
    name: String,
    ns: Option<String>,
    local: String,
    value: String,
}

fn ns_text(r: ResolveResult) -> Option<String> {
    match r {
        ResolveResult::Bound(n) => Some(String::from_utf8_lossy(n.as_ref()).into_owned()),
        _ => None,
    }
}

fn utf8(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Text of the packet: UTF-8, or UTF-16 with a byte order mark.
fn packet_text(data: &[u8]) -> String {
    let utf16 = |be: bool| {
        let units: Vec<u16> = data[2..]
            .chunks(2)
            .map(|c| {
                let pair = [c[0], *c.get(1).unwrap_or(&0)];
                if be {
                    u16::from_be_bytes(pair)
                } else {
                    u16::from_le_bytes(pair)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match data {
        [0xfe, 0xff, ..] => utf16(true),
        [0xff, 0xfe, ..] => utf16(false),
        [0xef, 0xbb, 0xbf, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(data).into_owned(),
    }
}

fn entity(name: &str) -> Option<char> {
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        _ => return None,
    })
}

/// Parse a packet, keeping at most `max_nodes` nodes of the flat tree.
pub fn parse(data: &[u8], max_nodes: usize) -> Xmp {
    let text = packet_text(data);
    let mut r = NsReader::from_str(&text);
    let mut stack: Vec<El> = Vec::new();
    let mut roots: Vec<El> = Vec::new();
    let mut error = None;
    let open = |r: &NsReader<&[u8]>, e: &quick_xml::events::BytesStart| {
        let (ns, local) = r.resolver().resolve_element(e.name());
        let attrs = e
            .attributes()
            .with_checks(false)
            .filter_map(Result::ok)
            .filter(|a: &Attribute| {
                let k = a.key.as_ref();
                k != b"xmlns" && !k.starts_with(b"xmlns:")
            })
            .map(|a| {
                let (ns, local) = r.resolver().resolve_attribute(QName(a.key.as_ref()));
                At {
                    name: utf8(a.key.as_ref()),
                    ns: ns_text(ns),
                    local: utf8(local.as_ref()),
                    value: a
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .map(|v| v.into_owned())
                        .unwrap_or_else(|_| utf8(&a.value)),
                }
            })
            .collect();
        El {
            name: utf8(e.name().as_ref()),
            ns: ns_text(ns),
            local: utf8(local.as_ref()),
            attrs,
            children: Vec::new(),
            text: String::new(),
        }
    };
    let close = |stack: &mut Vec<El>, roots: &mut Vec<El>, el: El| match stack.last_mut() {
        Some(p) => p.children.push(el),
        None => roots.push(el),
    };
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                if stack.len() >= MAX_DEPTH {
                    error = Some("elements nested too deeply".into());
                    break;
                }
                let el = open(&r, &e);
                stack.push(el);
            }
            Ok(Event::Empty(e)) => {
                let el = open(&r, &e);
                close(&mut stack, &mut roots, el);
            }
            Ok(Event::End(_)) => {
                if let Some(el) = stack.pop() {
                    close(&mut stack, &mut roots, el);
                }
            }
            Ok(Event::Text(t)) => {
                if let Some(top) = stack.last_mut() {
                    top.text
                        .push_str(&t.xml10_content().unwrap_or_else(|_| utf8(&t).into()));
                }
            }
            Ok(Event::CData(t)) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&utf8(&t));
                }
            }
            Ok(Event::GeneralRef(g)) => {
                let c = match g.resolve_char_ref() {
                    Ok(Some(c)) => Some(c),
                    _ => entity(&utf8(&g)),
                };
                if let (Some(top), Some(c)) = (stack.last_mut(), c) {
                    top.text.push(c);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => {
                error = Some(format!("XML error at byte {}: {e}", r.error_position()));
                break;
            }
        }
    }
    // Keep what was read of unclosed elements.
    while let Some(el) = stack.pop() {
        if error.is_none() {
            error = Some(format!("<{}> is not closed", el.name));
        }
        close(&mut stack, &mut roots, el);
    }

    let mut out = Xmp {
        error,
        ..Default::default()
    };
    for el in &roots {
        flatten(el, 0, max_nodes, &mut out);
        properties(el, &mut out.properties);
    }
    out
}

fn flatten(el: &El, depth: u32, max: usize, out: &mut Xmp) {
    if out.nodes.len() >= max {
        out.truncated = true;
        return;
    }
    let text = el.text.trim();
    out.nodes.push(XmpNode {
        depth,
        kind: NodeKind::Element,
        name: el.name.clone(),
        namespace: el.ns.clone(),
        value: (el.children.is_empty() && !text.is_empty()).then(|| text.to_string()),
    });
    for a in &el.attrs {
        if out.nodes.len() >= max {
            out.truncated = true;
            return;
        }
        out.nodes.push(XmpNode {
            depth: depth + 1,
            kind: NodeKind::Attribute,
            name: a.name.clone(),
            namespace: a.ns.clone(),
            value: Some(a.value.clone()),
        });
    }
    for c in &el.children {
        flatten(c, depth + 1, max, out);
    }
}

fn is_rdf(ns: &Option<String>, local: &str, want: &str) -> bool {
    ns.as_deref() == Some(RDF) && local == want
}

/// Properties of every `rdf:Description`, as attributes or child elements.
fn properties(el: &El, out: &mut Vec<XmpProperty>) {
    if is_rdf(&el.ns, &el.local, "Description") {
        for a in &el.attrs {
            match a.ns.as_deref() {
                Some(ns) if ns != RDF && !a.name.starts_with("xml:") => out.push(XmpProperty {
                    name: a.name.clone(),
                    namespace: ns.to_string(),
                    local: a.local.clone(),
                    value: a.value.clone(),
                }),
                _ => {}
            }
        }
        for c in &el.children {
            if let Some(ns) = c.ns.as_deref().filter(|&n| n != RDF) {
                out.push(XmpProperty {
                    name: c.name.clone(),
                    namespace: ns.to_string(),
                    local: c.local.clone(),
                    value: value_text(c),
                });
            }
        }
        return;
    }
    for c in &el.children {
        properties(c, out);
    }
}

/// A property's value as text: arrays joined (`rdf:Alt`: the default
/// language), resources by URI.
fn value_text(p: &El) -> String {
    if let Some(r) = p.attrs.iter().find(|a| is_rdf(&a.ns, &a.local, "resource")) {
        return r.value.clone();
    }
    for c in &p.children {
        let items = || c.children.iter().filter(|i| is_rdf(&i.ns, &i.local, "li"));
        if is_rdf(&c.ns, &c.local, "Alt") {
            let lang = |i: &&El| {
                i.attrs
                    .iter()
                    .any(|a| a.name == "xml:lang" && a.value == "x-default")
            };
            return items()
                .find(lang)
                .or_else(|| items().next())
                .map(|i| i.text.trim().to_string())
                .unwrap_or_default();
        }
        if is_rdf(&c.ns, &c.local, "Seq") || is_rdf(&c.ns, &c.local, "Bag") {
            return items()
                .map(|i| i.text.trim())
                .collect::<Vec<_>>()
                .join("; ");
        }
    }
    if p.children.is_empty() {
        p.text.trim().to_string()
    } else {
        "(structure)".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACKET: &str = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:CreatorTool="Tool &amp; Co">
   <dc:title xmlns:dc="http://purl.org/dc/elements/1.1/"><rdf:Alt><rdf:li xml:lang="en">English</rdf:li><rdf:li xml:lang="x-default">Default &#x263A;</rdf:li></rdf:Alt></dc:title>
   <dc:creator xmlns:dc="http://purl.org/dc/elements/1.1/"><rdf:Seq><rdf:li>A</rdf:li><rdf:li>B</rdf:li></rdf:Seq></dc:creator>
   <xmp:CreateDate>2024-01-31T12:00:00Z</xmp:CreateDate>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;

    #[test]
    fn properties_and_tree() {
        let x = parse(PACKET.as_bytes(), 1000);
        assert_eq!(x.error, None);
        assert_eq!(x.property(XMP, "CreatorTool"), Some("Tool & Co"));
        assert_eq!(x.property(DC, "title"), Some("Default ☺"));
        assert_eq!(x.property(DC, "creator"), Some("A; B"));
        assert_eq!(x.property(XMP, "CreateDate"), Some("2024-01-31T12:00:00Z"));
        assert_eq!(x.nodes[0].name, "x:xmpmeta");
        assert!(
            x.nodes
                .iter()
                .any(|n| n.name == "rdf:li" && n.value.as_deref() == Some("English"))
        );
        assert_eq!(parse(PACKET.as_bytes(), 3).nodes.len(), 3);
        assert!(parse(PACKET.as_bytes(), 3).truncated);
    }

    #[test]
    fn broken_xml_keeps_what_was_read() {
        let x = parse(b"<a><b>text</b><c>", 100);
        assert!(x.error.is_some());
        assert_eq!(x.nodes.len(), 3);
    }
}
