//! What a macro declares about itself (`export const meta`), as the prelude
//! checks it.

use serde::{Deserialize, Serialize};

/// The `meta` of a macro, with defaults filled in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    /// The prelude API the macro is written for.
    pub api: u32,
    pub name: String,
    pub title: String,
    pub description: String,
    /// Whether it changes the document (and so needs saving).
    pub edits: bool,
    /// How the host should save it.
    pub save: SaveSpec,
    /// Parameters in declaration order.
    #[serde(with = "ordered")]
    pub params: Vec<(String, Param)>,
}

impl Meta {
    pub fn param(&self, name: &str) -> Option<&Param> {
        self.params.iter().find(|(k, _)| k == name).map(|(_, p)| p)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SaveSpec {
    /// `"incremental"` or `"full"`.
    Mode(SaveKind),
    /// A full rewrite with options.
    Full(FullOptions),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SaveKind {
    Incremental,
    Full,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FullOptions {
    pub xref_stream: bool,
    pub object_streams: bool,
    pub compress: bool,
    pub remove_unused: bool,
}

impl SaveSpec {
    pub fn mode(&self) -> pdfi_api::SaveMode {
        let full = |o: FullOptions| pdfi_api::SaveMode::Full {
            xref_stream: o.xref_stream,
            object_streams: o.object_streams,
            compress: o.compress,
            remove_unused: o.remove_unused,
        };
        match *self {
            SaveSpec::Mode(SaveKind::Incremental) => pdfi_api::SaveMode::Incremental,
            SaveSpec::Mode(SaveKind::Full) => full(FullOptions::default()),
            SaveSpec::Full(o) => full(o),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParamType {
    String,
    Number,
    Integer,
    Boolean,
    Enum,
    /// `"1-3,8"` (1-based) given, 0-based page numbers received.
    Pages,
    Json,
    /// The contents of a file, as text.
    Text,
    /// The contents of a file.
    Bytes,
    /// Another document, opened read-only.
    Pdf,
}

impl ParamType {
    pub fn as_str(self) -> &'static str {
        match self {
            ParamType::String => "string",
            ParamType::Number => "number",
            ParamType::Integer => "integer",
            ParamType::Boolean => "boolean",
            ParamType::Enum => "enum",
            ParamType::Pages => "pages",
            ParamType::Json => "json",
            ParamType::Text => "text",
            ParamType::Bytes => "bytes",
            ParamType::Pdf => "pdf",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Param {
    #[serde(rename = "type")]
    pub ty: ParamType,
    #[serde(default)]
    pub optional: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
}

/// Parameters as a JSON object in declaration order.
mod ordered {
    use serde::de::{MapAccess, Visitor};
    use serde::ser::SerializeMap;
    use serde::{Deserializer, Serializer};

    use super::Param;

    pub fn serialize<S: Serializer>(v: &[(String, Param)], s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(Some(v.len()))?;
        for (k, p) in v {
            m.serialize_entry(k, p)?;
        }
        m.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<(String, Param)>, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Vec<(String, Param)>;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("parameters by name")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some((k, v)) = a.next_entry()? {
                    out.push((k, v));
                }
                Ok(out)
            }
        }
        d.deserialize_map(V)
    }
}
