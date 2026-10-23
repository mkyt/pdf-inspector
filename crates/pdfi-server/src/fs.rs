//! Filesystem access limited to what the user allowed at startup.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use std::io::Write;

use pdfi_api::{
    ApiError, DirEntry, DirListing, ErrorCode, OpenSource, Opened, SaveTarget, SourceProvider,
};
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

/// A file written next to its destination and renamed over it on commit.
struct TempFile {
    tmp: PathBuf,
    dest: PathBuf,
    file: Option<std::io::BufWriter<std::fs::File>>,
}

impl SaveTarget for TempFile {
    fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        match &mut self.file {
            Some(f) => f.write_all(data),
            None => Err(std::io::Error::other("already committed")),
        }
    }

    fn commit(mut self: Box<Self>) -> Result<Opened, ApiError> {
        let f = self
            .file
            .take()
            .ok_or_else(|| ApiError::new(ErrorCode::Internal, "already committed"))?;
        let f = f.into_inner().map_err(|e| e.into_error())?;
        f.sync_all()?;
        drop(f);
        // Replaces the old file atomically; a reader of the old one keeps it.
        std::fs::rename(&self.tmp, &self.dest)?;
        let src = FileSource::open(&self.dest)?;
        Ok(Opened {
            source: Arc::new(src),
            name: self
                .dest
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: Some(display(&self.dest)),
        })
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if self.file.take().is_some() {
            let _ = std::fs::remove_file(&self.tmp);
        }
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

    fn create(&self, path: &str) -> Result<Box<dyn SaveTarget>, ApiError> {
        let p = Path::new(path);
        let name = p
            .file_name()
            .filter(|n| !n.is_empty())
            .ok_or_else(|| {
                ApiError::new(ErrorCode::BadRequest, format!("{path} is not a file name"))
            })?
            .to_owned();
        // An existing file: the file itself must be allowed. A new one: its
        // directory must be under a root.
        let dest = match std::fs::canonicalize(p) {
            Ok(c) => {
                if !self.allowed_file(&c) {
                    return Err(Self::forbidden(path));
                }
                if !c.is_file() {
                    return Err(ApiError::new(
                        ErrorCode::BadRequest,
                        format!("{path} is not a file"),
                    ));
                }
                c
            }
            Err(_) => {
                let parent = p.parent().filter(|d| !d.as_os_str().is_empty());
                let dir = self.resolve(&parent.map_or(".".into(), display))?;
                if !self.under_root(&dir) || !dir.is_dir() {
                    return Err(Self::forbidden(path));
                }
                dir.join(&name)
            }
        };
        let dir = dest
            .parent()
            .ok_or_else(|| Self::forbidden(path))?
            .to_path_buf();
        let mut rnd = [0u8; 8];
        getrandom::fill(&mut rnd).map_err(|e| ApiError::new(ErrorCode::Internal, e.to_string()))?;
        let tag: String = rnd.iter().map(|b| format!("{b:02x}")).collect();
        let tmp = dir.join(format!(".{}.{tag}.pdfi-tmp", name.to_string_lossy()));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        Ok(Box::new(TempFile {
            tmp,
            dest,
            file: Some(std::io::BufWriter::with_capacity(1 << 20, file)),
        }))
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
