//! Where macros are found: the user's directories, then the bundled ones.

use std::path::{Path, PathBuf};

use crate::meta::Meta;
use crate::{BUNDLED, MacroError, read_meta};

/// Where a macro comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    Bundled,
    File(PathBuf),
}

/// A macro's code.
#[derive(Clone, Debug)]
pub struct MacroSource {
    /// The file name without `.js`.
    pub stem: String,
    /// Shown in stack traces and messages.
    pub file: String,
    pub origin: Origin,
    pub code: String,
}

impl MacroSource {
    pub fn read(path: &Path) -> Result<Self, MacroError> {
        let code = std::fs::read_to_string(path).map_err(|e| {
            MacroError::Io(std::io::Error::new(
                e.kind(),
                format!("{}: {e}", path.display()),
            ))
        })?;
        Ok(Self {
            stem: stem(path),
            file: path.display().to_string(),
            origin: Origin::File(path.to_path_buf()),
            code,
        })
    }

    fn bundled(file: &str, code: &str) -> Self {
        Self {
            stem: file.trim_end_matches(".js").into(),
            file: format!("bundled:{file}"),
            origin: Origin::Bundled,
            code: code.into(),
        }
    }

    /// Evaluate it without a document and read its `meta`.
    pub fn meta(&self) -> Result<Meta, MacroError> {
        read_meta(&self.file, &self.code, &self.stem)
    }
}

fn stem(p: &Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// One entry of [`Library::list`].
pub struct Listed {
    pub source: MacroSource,
    /// `Err` when it cannot be evaluated or its `meta` is wrong.
    pub meta: Result<Meta, MacroError>,
    /// Hidden by a macro of the same name found before it.
    pub shadowed: bool,
}

impl Listed {
    pub fn name(&self) -> &str {
        match &self.meta {
            Ok(m) => &m.name,
            Err(_) => &self.source.stem,
        }
    }
}

/// The user's macro directories, searched in order before the bundled
/// macros.
#[derive(Clone, Debug)]
pub struct Library {
    dirs: Vec<PathBuf>,
}

impl Library {
    pub fn new(dirs: Vec<PathBuf>) -> Self {
        Self { dirs }
    }

    /// `$PDFI_MACRO_PATH` (`:`-separated), or [`Library::default_dir`].
    pub fn from_env() -> Self {
        match std::env::var_os("PDFI_MACRO_PATH") {
            Some(p) if !p.is_empty() => Self::new(
                std::env::split_paths(&p)
                    .filter(|p| !p.as_os_str().is_empty())
                    .collect(),
            ),
            _ => Self::new(Self::default_dir().into_iter().collect()),
        }
    }

    /// `~/Library/Application Support/pdf-inspector/macros` on macOS,
    /// `$XDG_CONFIG_HOME/pdf-inspector/macros` (`~/.config/...`) elsewhere.
    pub fn default_dir() -> Option<PathBuf> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let base = if cfg!(target_os = "macos") {
            home?.join("Library/Application Support")
        } else {
            match std::env::var_os("XDG_CONFIG_HOME") {
                Some(x) if !x.is_empty() => PathBuf::from(x),
                _ => home?.join(".config"),
            }
        };
        Some(base.join("pdf-inspector/macros"))
    }

    pub fn dirs(&self) -> &[PathBuf] {
        &self.dirs
    }

    /// `*.js` in the user's directories (sorted by name in each), those
    /// that are missing skipped.
    fn files(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for d in &self.dirs {
            let Ok(rd) = std::fs::read_dir(d) else {
                continue;
            };
            let mut fs: Vec<PathBuf> = rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "js") && p.is_file())
                .collect();
            fs.sort();
            out.extend(fs);
        }
        out
    }

    /// Every macro: the user's first, then the bundled ones. A name found
    /// again later is marked shadowed.
    pub fn list(&self) -> Vec<Listed> {
        let mut out: Vec<Listed> = Vec::new();
        let sources = self
            .files()
            .into_iter()
            .map(|p| MacroSource::read(&p))
            .chain(BUNDLED.iter().map(|(f, c)| Ok(MacroSource::bundled(f, c))));
        for s in sources {
            let Ok(source) = s else { continue };
            let meta = source.meta();
            let mut l = Listed {
                source,
                meta,
                shadowed: false,
            };
            l.shadowed = out.iter().any(|o| o.name() == l.name());
            out.push(l);
        }
        out
    }

    /// A macro by path (anything with a `/` or ending in `.js`), or by
    /// name: a file `<name>.js` of a user directory, one whose
    /// `meta.name` is `name`, then a bundled macro.
    pub fn resolve(&self, what: &str) -> Result<(MacroSource, Meta), MacroError> {
        if what.contains('/') || what.ends_with(".js") {
            let s = MacroSource::read(Path::new(what))?;
            let m = s.meta()?;
            return Ok((s, m));
        }
        self.find(what)
    }

    /// A macro by name only (never a path): a file `<name>.js` of a user
    /// directory, one whose `meta.name` is `name`, then a bundled macro.
    pub fn find(&self, what: &str) -> Result<(MacroSource, Meta), MacroError> {
        check_name(what)?;
        for d in &self.dirs {
            let p = d.join(format!("{what}.js"));
            if p.is_file() {
                let s = MacroSource::read(&p)?;
                let m = s.meta()?;
                return Ok((s, m));
            }
        }
        for p in self.files() {
            let Ok(s) = MacroSource::read(&p) else {
                continue;
            };
            if let Ok(m) = s.meta()
                && m.name == what
            {
                return Ok((s, m));
            }
        }
        for (f, c) in BUNDLED {
            let s = MacroSource::bundled(f, c);
            let m = s.meta()?;
            if m.name == what || s.stem == what {
                return Ok((s, m));
            }
        }
        Err(MacroError::NotFound(format!(
            "no macro {what:?} (see `pdfi macro list`)"
        )))
    }
}

/// A name that is a file name of its own: letters, digits, `.`, `_`, `-`,
/// not starting with `.`.
pub fn check_name(name: &str) -> Result<(), MacroError> {
    let ok = !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(MacroError::Invalid(format!(
            "{name:?} is not a macro name (letters, digits, '.', '_' and '-')"
        )))
    }
}

impl Library {
    /// The directory [`Library::save`] writes into: the first one.
    pub fn save_dir(&self) -> Option<&Path> {
        self.dirs.first().map(PathBuf::as_path)
    }

    /// Write `<stem>.js` into the first directory (made if missing),
    /// replacing it; returns the stored macro.
    pub fn save(&self, stem: &str, code: &str) -> Result<Listed, MacroError> {
        let stem = stem.strip_suffix(".js").unwrap_or(stem);
        check_name(stem)?;
        let dir = self
            .save_dir()
            .ok_or_else(|| MacroError::Host("there is no macro directory to save into".into()))?;
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("{stem}.js"));
        let tmp = dir.join(format!(".{stem}.js.pdfi-tmp"));
        std::fs::write(&tmp, code)?;
        std::fs::rename(&tmp, &path)?;
        let source = MacroSource::read(&path)?;
        let meta = source.meta();
        let mut l = Listed {
            source,
            meta,
            shadowed: false,
        };
        let name = l.name().to_string();
        l.shadowed = self
            .list()
            .iter()
            .take_while(|o| o.source.origin != l.source.origin)
            .any(|o| o.name() == name);
        Ok(l)
    }

    /// Remove `<stem>.js` from the first user directory that has it.
    pub fn delete(&self, stem: &str) -> Result<(), MacroError> {
        let stem = stem.strip_suffix(".js").unwrap_or(stem);
        check_name(stem)?;
        for d in &self.dirs {
            let p = d.join(format!("{stem}.js"));
            if p.is_file() {
                std::fs::remove_file(&p)?;
                return Ok(());
            }
        }
        Err(MacroError::NotFound(format!(
            "no macro file {stem}.js in the macro directories"
        )))
    }
}
