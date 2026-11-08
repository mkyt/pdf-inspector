//! Parameters given on a command line (`k=v`, `k=@file`, a JSON file)
//! turned into the raw parameters [`crate::run`] takes. Conversion of the
//! strings and the checks are the prelude's; files are read here.

use std::path::{Path, PathBuf};

use pdfi_api::DocId;
use serde_json::{Map, Value};

use crate::MacroError;
use crate::meta::{Meta, ParamType};

/// The value of one `--param`.
#[derive(Clone, Debug, PartialEq)]
pub enum Given {
    Text(String),
    /// `@path`: the contents of a file (a document for `pdf`).
    File(PathBuf),
}

/// `key=value`, `key=@file` (`@@` for a value starting with `@`).
pub fn parse_arg(s: &str) -> Result<(String, Given), MacroError> {
    let Some((k, v)) = s.split_once('=') else {
        return Err(MacroError::Invalid(format!(
            "--param {s:?}: expected NAME=VALUE or NAME=@FILE"
        )));
    };
    let k = k.trim().to_string();
    let g = if let Some(rest) = v.strip_prefix("@@") {
        Given::Text(format!("@{rest}"))
    } else if let Some(p) = v.strip_prefix('@') {
        Given::File(PathBuf::from(p))
    } else {
        Given::Text(v.to_string())
    };
    Ok((k, g))
}

fn read(p: &Path) -> Result<Vec<u8>, MacroError> {
    std::fs::read(p).map_err(|e| {
        MacroError::Io(std::io::Error::new(
            e.kind(),
            format!("{}: {e}", p.display()),
        ))
    })
}

fn text(k: &str, p: &Path) -> Result<String, MacroError> {
    String::from_utf8(read(p)?).map_err(|_| {
        MacroError::Invalid(format!("parameter {k}: {} is not UTF-8 text", p.display()))
    })
}

pub fn base64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        s.push(A[(n >> 18) as usize & 63] as char);
        s.push(A[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 {
            A[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            A[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

/// The raw parameters from a JSON object (`--params`, taken as is) and
/// `--param`s (which win), files read by the declared type. `open_pdf`
/// opens a document for a `pdf` parameter and returns its id.
pub fn collect(
    meta: &Meta,
    file: Option<Map<String, Value>>,
    args: &[(String, Given)],
    open_pdf: &mut dyn FnMut(&Path) -> Result<DocId, MacroError>,
) -> Result<Map<String, Value>, MacroError> {
    let mut out = Map::new();
    let unknown = |k: &str| {
        let known: Vec<&str> = meta.params.iter().map(|(k, _)| k.as_str()).collect();
        MacroError::Invalid(format!(
            "parameter {k}: unknown (the macro takes {})",
            if known.is_empty() {
                "none".to_string()
            } else {
                known.join(", ")
            }
        ))
    };
    for (k, v) in file.unwrap_or_default() {
        let p = meta.param(&k).ok_or_else(|| unknown(&k))?;
        let v = match (p.ty, v) {
            (ParamType::Pdf, Value::String(path)) => {
                serde_json::json!({ "$doc": open_pdf(Path::new(&path))? })
            }
            (_, v) => v,
        };
        out.insert(k, v);
    }
    for (k, g) in args {
        let p = meta.param(k).ok_or_else(|| unknown(k))?;
        let v = match (p.ty, g) {
            (ParamType::Pdf, Given::Text(s)) => {
                serde_json::json!({ "$doc": open_pdf(Path::new(s))? })
            }
            (ParamType::Pdf, Given::File(f)) => serde_json::json!({ "$doc": open_pdf(f)? }),
            (ParamType::Json, Given::Text(s)) => serde_json::from_str(s)
                .map_err(|e| MacroError::Invalid(format!("parameter {k}: not JSON: {e}")))?,
            (ParamType::Json, Given::File(f)) => {
                serde_json::from_str(&text(k, f)?).map_err(|e| {
                    MacroError::Invalid(format!("parameter {k}: {}: not JSON: {e}", f.display()))
                })?
            }
            (ParamType::Bytes, Given::File(f)) => {
                serde_json::json!({ "$bytes": base64(&read(f)?) })
            }
            (ParamType::Text, Given::File(f)) => Value::String(text(k, f)?),
            // A short value kept in a file: without its final newline.
            (_, Given::File(f)) => {
                let t = text(k, f)?;
                Value::String(
                    t.strip_suffix('\n')
                        .map(|t| t.strip_suffix('\r').unwrap_or(t))
                        .unwrap_or(&t)
                        .to_string(),
                )
            }
            (_, Given::Text(s)) => Value::String(s.clone()),
        };
        out.insert(k.clone(), v);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64() {
        assert_eq!(super::base64(b"hello"), "aGVsbG8=");
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(&[0, 1, 2, 255]), "AAEC/w==");
    }

    #[test]
    fn args() {
        use super::{Given, parse_arg};
        assert_eq!(
            parse_arg("a=1").unwrap(),
            ("a".into(), Given::Text("1".into()))
        );
        assert_eq!(
            parse_arg("a=@x.json").unwrap(),
            ("a".into(), Given::File("x.json".into()))
        );
        assert_eq!(
            parse_arg("a=@@x").unwrap(),
            ("a".into(), Given::Text("@x".into()))
        );
        assert_eq!(
            parse_arg("a=b=c").unwrap(),
            ("a".into(), Given::Text("b=c".into()))
        );
        assert!(parse_arg("a").is_err());
    }
}
