//! The inventory: every font and image with the pages using them, and the
//! reverse reference graph (who refers to an object). It needs every object,
//! so a [`Builder`] does it in bounded steps (a job).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use pdfi_syntax::{Dict, DictExt, ObjId, Object, PdfFile};

use crate::Model;
use crate::util::{get, int, name, name_str, resolve};

/// Source number standing for the trailer in the reference graph.
pub const TRAILER: u32 = u32::MAX;
/// Nesting followed inside one object, and of form XObjects.
const MAX_DEPTH: usize = 64;
const MAX_FORM_DEPTH: usize = 16;

const STANDARD_14: [&str; 14] = [
    "Courier",
    "Courier-Bold",
    "Courier-BoldOblique",
    "Courier-Oblique",
    "Helvetica",
    "Helvetica-Bold",
    "Helvetica-BoldOblique",
    "Helvetica-Oblique",
    "Times-Roman",
    "Times-Bold",
    "Times-BoldItalic",
    "Times-Italic",
    "Symbol",
    "ZapfDingbats",
];

const FONT_SUBTYPES: [&str; 7] = [
    "Type1",
    "MMType1",
    "TrueType",
    "Type3",
    "Type0",
    "CIDFontType0",
    "CIDFontType2",
];

#[derive(Clone, Debug, PartialEq)]
pub struct FontEntry {
    pub id: ObjId,
    pub base_font: Option<String>,
    pub subtype: Option<String>,
    pub encoding: Option<String>,
    /// `FontFile`, `FontFile2`, `FontFile3/Type1C`, ... (of the descendant
    /// for Type 0 fonts); `None` when not embedded.
    pub embedded: Option<String>,
    pub font_file: Option<ObjId>,
    pub to_unicode: bool,
    /// First descendant of a Type 0 font.
    pub descendant: Option<ObjId>,
    /// Name has a subset tag (`ABCDEF+`).
    pub subset: bool,
    /// One of the standard 14 fonts (by name).
    pub standard: bool,
    /// Pages using it through their resources (0-based, ascending).
    pub pages: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageEntry {
    pub id: ObjId,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub bits: Option<i64>,
    pub color_space: Option<String>,
    pub filters: Vec<String>,
    pub raw_len: u64,
    pub image_mask: bool,
    pub smask: Option<ObjId>,
    pub mask: bool,
    pub interpolate: bool,
    pub pages: Vec<u32>,
}

/// Who refers to whom: for each object number, the numbers of the objects
/// (or [`TRAILER`]) whose values contain a reference to it.
#[derive(Clone, Debug, Default)]
pub struct RefIndex {
    starts: Vec<u32>,
    sources: Vec<u32>,
}

impl RefIndex {
    fn build(mut pairs: Vec<(u32, u32)>, slots: u32) -> Self {
        pairs.sort_unstable();
        pairs.dedup();
        let n = slots as usize + 1;
        let mut starts = vec![0u32; n + 1];
        let mut sources = Vec::with_capacity(pairs.len());
        for &(t, s) in &pairs {
            if (t as usize) < n {
                starts[t as usize + 1] += 1;
                sources.push(s);
            }
        }
        for i in 0..n {
            starts[i + 1] += starts[i];
        }
        Self { starts, sources }
    }

    pub fn referrers(&self, num: u32) -> &[u32] {
        let i = num as usize;
        match (self.starts.get(i), self.starts.get(i + 1)) {
            (Some(&a), Some(&b)) => &self.sources[a as usize..b as usize],
            _ => &[],
        }
    }

    pub fn edges(&self) -> usize {
        self.sources.len()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Inventory {
    pub fonts: Vec<FontEntry>,
    pub images: Vec<ImageEntry>,
    pub forms: u64,
    /// Objects scanned, and those that failed to load.
    pub objects: u64,
    pub failed: u64,
    pub pages: u64,
    pub refs: RefIndex,
    /// Live objects nothing refers to (object and cross-reference streams
    /// and linearization dictionaries aside).
    pub unreferenced: Vec<ObjId>,
    /// Live objects grouped by kind, `/Type` and `/Subtype` (and schema
    /// type, see [`ObjClass::schema_type`]), most common first.
    pub classes: Vec<ObjClass>,
}

/// Objects of one kind (`dictionary`, `stream`, ...; [`UNLOADABLE`] for
/// those that failed to load) with the same `/Type` and `/Subtype` (and
/// schema type, once a caller that knows the schema split them by it).
#[derive(Clone, Debug, Default)]
pub struct ObjClass {
    pub kind: &'static str,
    pub type_name: Option<String>,
    pub subtype: Option<String>,
    /// Not set by [`Builder`], which knows nothing of the schema.
    pub schema_type: Option<String>,
    /// Ascending.
    pub ids: Vec<ObjId>,
}

/// Kind of the [`ObjClass`] of objects that failed to load.
pub const UNLOADABLE: &str = "unloadable";

type ClassKey = (&'static str, Option<String>, Option<String>);

/// Builds an [`Inventory`]: first every object (fonts, images, references),
/// then every page (which fonts and images it uses).
pub struct Builder {
    ids: Vec<ObjId>,
    pos: usize,
    pages: u64,
    page_pos: u64,
    slots: u32,
    pairs: Vec<(u32, u32)>,
    fonts: BTreeMap<ObjId, FontEntry>,
    images: BTreeMap<ObjId, ImageEntry>,
    /// Fonts and images a form XObject uses (with nested forms).
    reach: HashMap<ObjId, Arc<Vec<ObjId>>>,
    forms: u64,
    failed: HashSet<u32>,
    structural: HashSet<u32>,
    classes: HashMap<ClassKey, Vec<ObjId>>,
}

fn class_key(o: &Object) -> ClassKey {
    let d = match o {
        Object::Dict(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    };
    let get = |k: &str| d.and_then(|d| d.get_name(k)).map(name_str);
    (o.type_name(), get("Type"), get("Subtype"))
}

fn collect_refs(o: &Object, src: u32, depth: usize, out: &mut Vec<(u32, u32)>) {
    if depth > MAX_DEPTH {
        return;
    }
    match o {
        Object::Ref(id) => out.push((id.num, src)),
        Object::Array(a) => a
            .iter()
            .for_each(|n| collect_refs(&n.obj, src, depth + 1, out)),
        Object::Dict(d) => d
            .values()
            .for_each(|n| collect_refs(&n.obj, src, depth + 1, out)),
        Object::Stream(s) => s
            .dict
            .values()
            .for_each(|n| collect_refs(&n.obj, src, depth + 1, out)),
        _ => {}
    }
}

fn is_font(d: &Dict) -> bool {
    d.type_is("Font")
        || (d
            .get_name("Subtype")
            .is_some_and(|s| FONT_SUBTYPES.iter().any(|t| s == t))
            && (d.get_key("BaseFont").is_some() || d.get_key("FontDescriptor").is_some()))
}

/// `/DeviceRGB`, `ICCBased (3)`, `Indexed DeviceRGB 255`, ...
pub fn color_space_text(file: &PdfFile, o: &Object) -> String {
    let r = resolve(file, o).1;
    match &*r {
        Object::Name(n) => name_str(n),
        Object::Array(a) => {
            let Some(head) = a
                .first()
                .and_then(|h| resolve(file, &h.obj).1.as_name().cloned())
            else {
                return "array".into();
            };
            let head = name_str(&head);
            let arg = |i: usize| a.get(i).map(|n| resolve(file, &n.obj).1);
            match head.as_str() {
                "ICCBased" => {
                    match arg(1).and_then(|s| s.as_dict().and_then(|d| int(file, d, "N"))) {
                        Some(n) => format!("ICCBased ({n})"),
                        None => head,
                    }
                }
                "Indexed" | "I" => {
                    let base = a
                        .get(1)
                        .map(|b| color_space_text(file, &b.obj))
                        .unwrap_or_default();
                    let hival = arg(2).and_then(|h| h.as_i64()).unwrap_or(0);
                    format!("Indexed {base} {hival}")
                }
                "Separation" => match arg(1).and_then(|n| n.as_name().cloned()) {
                    Some(n) => format!("Separation /{}", name_str(&n)),
                    None => head,
                },
                "DeviceN" => match arg(1).and_then(|n| n.as_array().map(<[_]>::len)) {
                    Some(n) => format!("DeviceN ({n})"),
                    None => head,
                },
                _ => head,
            }
        }
        o => o.type_name().into(),
    }
}

fn font_entry(file: &PdfFile, id: ObjId, d: &Dict) -> FontEntry {
    let base_font = name(file, d, "BaseFont").map(|n| name_str(&n));
    let subtype = name(file, d, "Subtype").map(|n| name_str(&n));
    let encoding = get(file, d, "Encoding").map(|e| match &*e {
        Object::Name(n) => format!("/{}", name_str(n)),
        Object::Stream(s) => match s.dict.get_name("CMapName") {
            Some(n) => format!("embedded CMap /{}", name_str(n)),
            None => "embedded CMap".into(),
        },
        Object::Dict(ed) => {
            let base = name(file, ed, "BaseEncoding").map(|n| format!("/{}", name_str(&n)));
            match (base, ed.get_key("Differences").is_some()) {
                (Some(b), true) => format!("{b} + Differences"),
                (Some(b), false) => b,
                (None, true) => "Differences".into(),
                (None, false) => "dictionary".into(),
            }
        }
        o => o.type_name().into(),
    });
    let descendant = get(file, d, "DescendantFonts").and_then(|a| {
        a.as_array()
            .and_then(|a| a.first())
            .and_then(|n| n.obj.as_ref_id())
    });
    // A Type 0 font's glyphs are in its descendant.
    let desc_font = descendant.and_then(|id| file.get(id).ok());
    let holder = desc_font
        .as_ref()
        .and_then(|o| o.node.obj.as_dict())
        .unwrap_or(d);
    let mut embedded = None;
    let mut font_file = None;
    if let Some(fd) = get(file, holder, "FontDescriptor") {
        if let Some(fd) = fd.as_dict() {
            for key in ["FontFile", "FontFile2", "FontFile3"] {
                let Some(v) = fd.get_key(key) else { continue };
                let (fid, s) = resolve(file, v);
                font_file = fid;
                embedded = Some(match s.as_dict().and_then(|sd| sd.get_name("Subtype")) {
                    Some(st) => format!("{key}/{}", name_str(st)),
                    None => key.to_string(),
                });
                break;
            }
        }
    }
    if subtype.as_deref() == Some("Type3") {
        embedded = Some("Type3 glyph procedures".into());
    }
    let plain = base_font
        .as_deref()
        .map(|b| match b.split_once('+') {
            Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|c| c.is_ascii_uppercase()) => {
                rest
            }
            _ => b,
        })
        .unwrap_or("");
    FontEntry {
        id,
        subset: base_font.as_deref().is_some_and(|b| b.len() > plain.len()),
        standard: STANDARD_14.contains(&plain),
        base_font,
        subtype,
        encoding,
        embedded,
        font_file,
        to_unicode: d.get_key("ToUnicode").is_some(),
        descendant,
        pages: Vec::new(),
    }
}

fn image_entry(file: &PdfFile, id: ObjId, s: &pdfi_syntax::Stream) -> ImageEntry {
    let d = &s.dict;
    let flag = |k: &str| get(file, d, k).is_some_and(|v| matches!(*v, Object::Bool(true)));
    ImageEntry {
        id,
        width: int(file, d, "Width"),
        height: int(file, d, "Height"),
        bits: int(file, d, "BitsPerComponent"),
        color_space: d.get_key("ColorSpace").map(|c| color_space_text(file, c)),
        filters: file
            .filter_specs(d)
            .iter()
            .map(|f| name_str(&f.name))
            .collect(),
        raw_len: match &s.data {
            pdfi_syntax::StreamData::Source(sp) => sp.len(),
            pdfi_syntax::StreamData::Owned(b) => b.len() as u64,
        },
        image_mask: flag("ImageMask"),
        smask: d.get_key("SMask").and_then(Object::as_ref_id),
        mask: d.get_key("Mask").is_some(),
        interpolate: flag("Interpolate"),
        pages: Vec::new(),
    }
}

impl Builder {
    pub fn new(m: &Model) -> Self {
        let f = m.file;
        let mut pairs = Vec::new();
        for n in f.trailer().values() {
            collect_refs(&n.obj, TRAILER, 0, &mut pairs);
        }
        Self {
            ids: f.xref().live_ids().collect(),
            pos: 0,
            pages: m.page_count(),
            page_pos: 0,
            slots: f.xref().size(),
            pairs,
            fonts: BTreeMap::new(),
            images: BTreeMap::new(),
            reach: HashMap::new(),
            forms: 0,
            failed: HashSet::new(),
            structural: HashSet::new(),
            classes: HashMap::new(),
        }
    }

    pub fn total(&self) -> u64 {
        self.ids.len() as u64 + self.pages
    }

    pub fn done(&self) -> u64 {
        self.pos as u64 + self.page_pos
    }

    /// Scan up to `objects` objects, or then up to `pages` pages. Returns
    /// whether work remains.
    pub fn step(&mut self, m: &Model, objects: usize, pages: usize) -> bool {
        let f = m.file;
        if self.pos < self.ids.len() {
            let end = (self.pos + objects).min(self.ids.len());
            for &id in &self.ids[self.pos..end] {
                let Ok(o) = f.get(id) else {
                    self.failed.insert(id.num);
                    self.classes
                        .entry((UNLOADABLE, None, None))
                        .or_default()
                        .push(id);
                    continue;
                };
                collect_refs(&o.node.obj, id.num, 0, &mut self.pairs);
                self.classes
                    .entry(class_key(&o.node.obj))
                    .or_default()
                    .push(id);
                match &o.node.obj {
                    Object::Dict(d) if is_font(d) => {
                        self.fonts.insert(o.id, font_entry(f, o.id, d));
                    }
                    Object::Dict(d) if d.get_key("Linearized").is_some() => {
                        self.structural.insert(id.num);
                    }
                    Object::Stream(s) => {
                        let d = &s.dict;
                        if d.type_is("ObjStm") || d.type_is("XRef") {
                            self.structural.insert(id.num);
                        } else if d.get_name("Subtype").is_some_and(|n| n == "Image") {
                            self.images.insert(o.id, image_entry(f, o.id, s));
                        } else if d.get_name("Subtype").is_some_and(|n| n == "Form") {
                            self.forms += 1;
                        }
                    }
                    _ => {}
                }
            }
            self.pos = end;
            return true;
        }
        let end = (self.page_pos + pages as u64).min(self.pages);
        for i in self.page_pos..end {
            let Some(res) = m.page(i).ok().and_then(|p| m.resources_of(&p)) else {
                continue;
            };
            let mut used = Vec::new();
            self.uses(f, &res, 0, &mut Vec::new(), &mut used);
            used.sort_unstable();
            used.dedup();
            for id in used {
                let page = i as u32;
                if let Some(e) = self.fonts.get_mut(&id) {
                    e.pages.push(page);
                } else if let Some(e) = self.images.get_mut(&id) {
                    e.pages.push(page);
                }
            }
        }
        self.page_pos = end;
        self.page_pos < self.pages
    }

    /// Fonts and images a resource dictionary uses, also through form
    /// XObjects. Fonts without `/Type` are added to the inventory here.
    fn uses(
        &mut self,
        f: &PdfFile,
        res: &Object,
        depth: usize,
        forms: &mut Vec<ObjId>,
        out: &mut Vec<ObjId>,
    ) {
        let r = resolve(f, res).1;
        let Some(res) = r.as_dict() else { return };
        if let Some(fonts) = get(f, res, "Font") {
            for v in fonts.as_dict().into_iter().flat_map(|d| d.values()) {
                let Some(id) = v.obj.as_ref_id() else {
                    continue;
                };
                if let std::collections::btree_map::Entry::Vacant(e) = self.fonts.entry(id) {
                    if let Some(o) = f.get(id).ok().filter(|o| o.node.obj.as_dict().is_some()) {
                        e.insert(font_entry(f, id, o.node.obj.as_dict().expect("dict")));
                    }
                }
                out.push(id);
            }
        }
        let Some(xobjects) = get(f, res, "XObject") else {
            return;
        };
        for v in xobjects.as_dict().into_iter().flat_map(|d| d.values()) {
            let Some(id) = v.obj.as_ref_id() else {
                continue;
            };
            if self.images.contains_key(&id) {
                out.push(id);
                continue;
            }
            if depth >= MAX_FORM_DEPTH || forms.contains(&id) {
                continue;
            }
            if let Some(r) = self.reach.get(&id) {
                out.extend(r.iter().copied());
                continue;
            }
            let Some(o) = f.get(id).ok() else { continue };
            let Some(s) = o.node.obj.as_stream() else {
                continue;
            };
            if !s.dict.get_name("Subtype").is_some_and(|n| n == "Form") {
                continue;
            }
            let mut inner = Vec::new();
            if let Some(r) = s.dict.get_key("Resources") {
                forms.push(id);
                self.uses(f, r, depth + 1, forms, &mut inner);
                forms.pop();
            }
            inner.sort_unstable();
            inner.dedup();
            out.extend(inner.iter().copied());
            self.reach.insert(id, Arc::new(inner));
        }
    }

    pub fn finish(self) -> Inventory {
        let refs = RefIndex::build(self.pairs, self.slots);
        let unreferenced = self
            .ids
            .iter()
            .filter(|id| {
                refs.referrers(id.num).is_empty()
                    && !self.structural.contains(&id.num)
                    && !self.failed.contains(&id.num)
            })
            .copied()
            .collect();
        let mut classes: Vec<ObjClass> = self
            .classes
            .into_iter()
            .map(|((kind, type_name, subtype), ids)| ObjClass {
                kind,
                type_name,
                subtype,
                schema_type: None,
                ids,
            })
            .collect();
        sort_classes(&mut classes);
        Inventory {
            fonts: self.fonts.into_values().collect(),
            images: self.images.into_values().collect(),
            forms: self.forms,
            objects: self.ids.len() as u64,
            failed: self.failed.len() as u64,
            pages: self.pages,
            refs,
            unreferenced,
            classes,
        }
    }
}

/// Most common first.
pub fn sort_classes(classes: &mut [ObjClass]) {
    classes.sort_by(|a, b| {
        b.ids.len().cmp(&a.ids.len()).then_with(|| {
            (a.kind, &a.type_name, &a.subtype, &a.schema_type).cmp(&(
                b.kind,
                &b.type_name,
                &b.subtype,
                &b.schema_type,
            ))
        })
    });
}

/// Where inside `o` it refers to `target`: paths like `/Resources/Font/F1`
/// (at most `max`).
pub fn ref_paths(o: &Object, target: ObjId, max: usize) -> Vec<String> {
    fn go(
        o: &Object,
        target: ObjId,
        path: &mut String,
        depth: usize,
        max: usize,
        out: &mut Vec<String>,
    ) {
        if out.len() >= max || depth > MAX_DEPTH {
            return;
        }
        let mut each = |seg: String, child: &Object, out: &mut Vec<String>| {
            let len = path.len();
            path.push_str(&seg);
            go(child, target, path, depth + 1, max, out);
            path.truncate(len);
        };
        match o {
            Object::Ref(id) if id.num == target.num => out.push(path.clone()),
            Object::Array(a) => {
                for (i, n) in a.iter().enumerate() {
                    each(format!("[{i}]"), &n.obj, out);
                }
            }
            Object::Dict(d) => {
                for (k, n) in d {
                    each(format!("/{}", name_str(k)), &n.obj, out);
                }
            }
            Object::Stream(s) => {
                for (k, n) in &s.dict {
                    each(format!("/{}", name_str(k)), &n.obj, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    go(o, target, &mut String::new(), 0, max, &mut out);
    out
}

/// One-line description of a resource or other object: `Type1 Helvetica`,
/// `Image 100×50 DeviceRGB`, `Form`, ...
pub fn describe(file: &PdfFile, o: &Object) -> String {
    let r = resolve(file, o).1;
    match &*r {
        Object::Dict(d) if is_font(d) => {
            let st = d.get_name("Subtype").map(name_str).unwrap_or_default();
            let bf = name(file, d, "BaseFont")
                .map(|n| name_str(&n))
                .unwrap_or_default();
            format!("{st} {bf}").trim().to_string()
        }
        Object::Stream(s) if s.dict.get_name("Subtype").is_some_and(|n| n == "Image") => {
            let d = &s.dict;
            let mut t = format!(
                "Image {}×{}",
                int(file, d, "Width").unwrap_or(0),
                int(file, d, "Height").unwrap_or(0)
            );
            if let Some(c) = d.get_key("ColorSpace") {
                t += &format!(" {}", color_space_text(file, c));
            }
            t
        }
        Object::Stream(s) if s.dict.get_name("Subtype").is_some_and(|n| n == "Form") => {
            "Form".into()
        }
        Object::Dict(d) | Object::Stream(pdfi_syntax::Stream { dict: d, .. }) => {
            match (d.get_name("Type"), d.get_name("Subtype")) {
                (Some(t), Some(s)) => format!("{} {}", name_str(t), name_str(s)),
                (Some(t), None) => name_str(t),
                (None, Some(s)) => name_str(s),
                (None, None) => r.type_name().into(),
            }
        }
        Object::Array(_) => color_space_text(file, &r),
        Object::Name(n) => format!("/{}", name_str(n)),
        o => o.type_name().into(),
    }
}
