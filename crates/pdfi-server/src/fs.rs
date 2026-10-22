//! Filesystem access limited to what the user allowed at startup.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pdfi_api::{ApiError, DirEntry, DirListing, ErrorCode, OpenSource, Opened, SourceProvider};
use pdfi_syntax::FileSource;

/// Allows the given files and everything below the given directories. Every
/// path is canonicalized (symlinks resolved) before it is checked.
#[derive(Debug, Clone)]
pub struct FsProvider {
    roots: Vec<PathBuf>,
    files: Vec<PathBuf>,
}

impl FsProvider {
    pub fn new(roots: &[PathBuf], files: &[PathBuf]) -> std::io::Result<Self> {
        let canon = |ps: &[PathBuf]| -> std::io::Result<Vec<PathBuf>> {
            ps.iter().map(std::fs::canonicalize).collect()
        };
        Ok(Self {
            roots: canon(roots)?,
            files: canon(files)?,
        })
    }

    fn forbidden(p: &str) -> ApiError {
        ApiError::new(
            ErrorCode::Forbidden,
            format!("{p} is outside the allowed paths"),
        )
    }

    fn resolve(&self, p: &str) -> Result<PathBuf, ApiError> {
        std::fs::canonicalize(p).map_err(|e| {
            // Do not reveal whether paths outside the roots exist.
            if !self.under_root(Path::new(p)) {
                Self::forbidden(p)
            } else if e.kind() == std::io::ErrorKind::NotFound {
                ApiError::new(ErrorCode::NotFound, format!("{p}: {e}"))
            } else {
                ApiError::new(ErrorCode::Io, format!("{p}: {e}"))
            }
        })
    }

    fn under_root(&self, c: &Path) -> bool {
        self.roots.iter().any(|r| c.starts_with(r))
    }

    fn allowed_file(&self, c: &Path) -> bool {
        self.files.iter().any(|f| f == c) || self.under_root(c)
    }
}

fn display(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn entry(name: String, p: &Path) -> DirEntry {
    let md = std::fs::metadata(p).ok();
    let is_dir = md.as_ref().is_some_and(|m| m.is_dir());
    DirEntry {
        name,
        path: display(p),
        is_dir,
        size: md.filter(|m| m.is_file()).map(|m| m.len()),
    }
}

impl SourceProvider for FsProvider {
    fn filesystem(&self) -> bool {
        true
    }

    fn open(&self, source: &OpenSource) -> Result<Opened, ApiError> {
        let OpenSource::Path { path } = source;
        let c = self.resolve(path)?;
        if !self.allowed_file(&c) {
            return Err(Self::forbidden(path));
        }
        if !c.is_file() {
            return Err(ApiError::new(
                ErrorCode::BadRequest,
                format!("{path} is not a file"),
            ));
        }
        let src = FileSource::open(&c)?;
        Ok(Opened {
            source: Arc::new(src),
            name: c
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: Some(display(&c)),
        })
    }

    fn list_dir(&self, path: Option<&str>) -> Result<DirListing, ApiError> {
        let Some(path) = path else {
            let entries = self
                .roots
                .iter()
                .chain(&self.files)
                .map(|p| entry(display(p), p))
                .collect();
            return Ok(DirListing {
                path: None,
                parent: None,
                entries,
            });
        };
        let c = self.resolve(path)?;
        if !self.under_root(&c) {
            return Err(Self::forbidden(path));
        }
        let mut entries: Vec<DirEntry> = std::fs::read_dir(&c)?
            .filter_map(|e| e.ok())
            .map(|e| entry(e.file_name().to_string_lossy().into_owned(), &e.path()))
            .collect();
        entries.sort_by(|a, b| (!a.is_dir, &a.name).cmp(&(!b.is_dir, &b.name)));
        let parent = c.parent().filter(|p| self.under_root(p)).map(display);
        Ok(DirListing {
            path: Some(display(&c)),
            parent,
            entries,
        })
    }
}
