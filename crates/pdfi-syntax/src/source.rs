//! Random-access byte sources.
//!
//! The reader never requires the whole file in memory: every access goes
//! through [`ByteSource::read_at`].

use std::io;
use std::sync::Arc;

pub trait ByteSource: Send + Sync {
    fn len(&self) -> u64;

    /// Fill `buf` with bytes starting at `offset`. Returns the number of bytes
    /// read, which is smaller than `buf.len()` only at the end of the source.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize>;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Read `len` bytes at `offset` (clamped to the end of the source).
pub fn read_vec(src: &dyn ByteSource, offset: u64, len: usize) -> io::Result<Vec<u8>> {
    let avail = src.len().saturating_sub(offset);
    let len = len.min(usize::try_from(avail).unwrap_or(usize::MAX));
    let mut buf = vec![0; len];
    let mut filled = 0;
    while filled < len {
        let n = src.read_at(offset + filled as u64, &mut buf[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    buf.truncate(filled);
    Ok(buf)
}

/// In-memory source.
#[derive(Clone, Debug)]
pub struct MemSource(Arc<[u8]>);

impl MemSource {
    pub fn new(data: impl Into<Arc<[u8]>>) -> Self {
        Self(data.into())
    }
}

impl ByteSource for MemSource {
    fn len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        let Ok(start) = usize::try_from(offset) else {
            return Ok(0);
        };
        if start >= self.0.len() {
            return Ok(0);
        }
        let n = buf.len().min(self.0.len() - start);
        buf[..n].copy_from_slice(&self.0[start..start + n]);
        Ok(n)
    }
}

/// File source using positional reads (no shared cursor, safe across threads).
#[cfg(any(unix, windows))]
#[derive(Debug)]
pub struct FileSource {
    file: std::fs::File,
    len: u64,
}

#[cfg(any(unix, windows))]
impl FileSource {
    pub fn open(path: impl AsRef<std::path::Path>) -> io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self { file, len })
    }
}

#[cfg(any(unix, windows))]
impl ByteSource for FileSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
        if offset >= self.len {
            return Ok(0);
        }
        let want = buf
            .len()
            .min(usize::try_from(self.len - offset).unwrap_or(usize::MAX));
        let buf = &mut buf[..want];
        let mut filled = 0;
        while filled < want {
            #[cfg(unix)]
            let n = std::os::unix::fs::FileExt::read_at(
                &self.file,
                &mut buf[filled..],
                offset + filled as u64,
            )?;
            #[cfg(windows)]
            let n = std::os::windows::fs::FileExt::seek_read(
                &self.file,
                &mut buf[filled..],
                offset + filled as u64,
            )?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        Ok(filled)
    }
}

/// `io::Read` over a byte range of a source.
pub struct RangeReader<'a> {
    src: &'a dyn ByteSource,
    pos: u64,
    end: u64,
}

impl<'a> RangeReader<'a> {
    pub fn new(src: &'a dyn ByteSource, start: u64, end: u64) -> Self {
        Self {
            src,
            pos: start,
            end: end.min(src.len()),
        }
    }
}

impl io::Read for RangeReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.end {
            return Ok(0);
        }
        let n = buf
            .len()
            .min(usize::try_from(self.end - self.pos).unwrap_or(usize::MAX));
        let n = self.src.read_at(self.pos, &mut buf[..n])?;
        self.pos += n as u64;
        Ok(n)
    }
}
