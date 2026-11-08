//! A [`MacroHost`] for a [`Session`]: macro jobs (`JobKind::Macro`) and
//! the stored-macro requests, over a [`Library`] (the server, macOS).

use std::sync::{Arc, Weak};

use pdfi_api::{
    ApiError, EmittedFile, ErrorCode, LogLevel, MacroHost, MacroInfo, MacroOutput, MacroResult,
    MacroRun, Session,
};

use crate::engine::{Level, RunOptions, run};
use crate::library::{Library, Listed, MacroSource, Origin};
use crate::{MacroError, read_meta};

/// Runs macros for one session from a library.
pub struct SessionHost {
    session: Weak<Session>,
    library: Library,
    host: String,
}

/// Make `session` run macros from `library`; `host` is what macros see
/// as `ctx.host` (`"server"`, `"macos"`).
pub fn install(session: &Arc<Session>, library: Library, host: &str) {
    session.set_macro_host(Arc::new(SessionHost {
        session: Arc::downgrade(session),
        library,
        host: host.into(),
    }));
}

fn api(e: MacroError) -> ApiError {
    let code = match &e {
        MacroError::Cancelled => ErrorCode::Cancelled,
        MacroError::NotFound(_) => ErrorCode::NotFound,
        MacroError::Invalid(_) | MacroError::Script { .. } | MacroError::Timeout => {
            ErrorCode::BadRequest
        }
        MacroError::Api(a) => a.code,
        MacroError::Io(_) => ErrorCode::Io,
        MacroError::Host(_) => ErrorCode::Internal,
    };
    ApiError::new(code, e.to_string())
}

fn info(l: &Listed) -> MacroInfo {
    MacroInfo {
        name: l.name().to_string(),
        stem: l.source.stem.clone(),
        file: l.source.file.clone(),
        bundled: l.source.origin == Origin::Bundled,
        meta: l
            .meta
            .as_ref()
            .ok()
            .map(|m| serde_json::to_string(m).unwrap()),
        error: l.meta.as_ref().err().map(|e| e.to_string()),
        shadowed: l.shadowed,
    }
}

impl MacroHost for SessionHost {
    fn list(&self) -> Result<(Vec<MacroInfo>, Option<String>), ApiError> {
        let items = self.library.list().iter().map(info).collect();
        let dir = self.library.save_dir().map(|d| d.display().to_string());
        Ok((items, dir))
    }

    fn source(&self, name: &str) -> Result<(MacroInfo, String), ApiError> {
        let (source, meta) = self.library.find(name).map_err(api)?;
        let l = Listed {
            source,
            meta: Ok(meta),
            shadowed: false,
        };
        Ok((info(&l), l.source.code.clone()))
    }

    fn save(&self, name: &str, code: &str) -> Result<MacroInfo, ApiError> {
        self.library.save(name, code).map(|l| info(&l)).map_err(api)
    }

    fn delete(&self, name: &str) -> Result<(), ApiError> {
        self.library.delete(name).map_err(api)
    }

    fn run(&self, r: &MacroRun) -> Result<MacroOutput, ApiError> {
        let session = self
            .session
            .upgrade()
            .ok_or_else(|| ApiError::new(ErrorCode::Internal, "the session is gone"))?;
        let (source, meta) = match &r.source {
            Some(code) => {
                let stem = r.name.strip_suffix(".js").unwrap_or(&r.name).to_string();
                let file = format!("{stem}.js");
                let meta = read_meta(&file, code, &stem).map_err(api)?;
                let source = MacroSource {
                    stem,
                    file,
                    origin: Origin::Bundled,
                    code: code.clone(),
                };
                (source, meta)
            }
            None => self.library.find(&r.name).map_err(api)?,
        };
        let params: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&r.params)
            .map_err(|e| {
                ApiError::new(
                    ErrorCode::BadRequest,
                    format!("parameters are not a JSON object: {e}"),
                )
            })?;
        let (l, p) = (r.reporter(), r.reporter());
        let level = |lv| match lv {
            Level::Info => LogLevel::Info,
            Level::Warn => LogLevel::Warn,
            Level::Error => LogLevel::Error,
        };
        let opts = RunOptions {
            cancel: r.cancel_flag(),
            timeout: None,
            host: self.host.clone(),
            input: serde_json::json!({ "name": r.input_name, "size": r.input_size }),
            log: Box::new(move |lv, text| l.log(level(lv), text)),
            progress: Box::new(move |done, total, label| p.progress(done, total, label)),
        };
        let out = run(
            &session,
            r.doc,
            &source.file,
            &source.code,
            &meta,
            &params,
            opts,
        );
        let out = out.map_err(api)?;
        Ok(MacroOutput {
            result: MacroResult {
                value: out.value.to_string(),
                columns: out.columns,
                emitted: out
                    .emitted
                    .iter()
                    .map(|e| EmittedFile {
                        name: e.name.clone(),
                        mime: e.mime.clone(),
                        size: e.data.len() as u64,
                    })
                    .collect(),
                log_truncated: r.reporter().log_truncated(),
                edits: meta.edits,
                save: meta.save.mode(),
            },
            files: out.emitted.into_iter().map(|e| e.data).collect(),
        })
    }
}
