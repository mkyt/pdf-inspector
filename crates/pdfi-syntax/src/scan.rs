//! Buffered forward search over a [`ByteSource`].

use std::io;

use memchr::memmem;

use crate::source::{ByteSource, read_vec};

const CHUNK: usize = 1 << 20;

pub struct ChunkScanner<'a> {
    src: &'a dyn ByteSource,
    buf: Vec<u8>,
    buf_start: u64,
}

impl<'a> ChunkScanner<'a> {
    pub fn new(src: &'a dyn ByteSource) -> Self {
        Self {
            src,
            buf: Vec::new(),
            buf_start: 0,
        }
    }

    /// Ensure the buffer covers `[at, at + min_len)` (or up to the end of the source).
    fn fill(&mut self, at: u64, min_len: usize) -> io::Result<()> {
        let end = self.buf_start + self.buf.len() as u64;
        if at >= self.buf_start && at + min_len as u64 <= end {
            return Ok(());
        }
        self.buf = read_vec(self.src, at, CHUNK.max(min_len))?;
        self.buf_start = at;
        Ok(())
    }

    /// First occurrence of any of `needles` at or after `from`:
    /// `(absolute offset, needle index)`.
    pub fn find_any(&mut self, from: u64, needles: &[&[u8]]) -> io::Result<Option<(u64, usize)>> {
        let longest = needles.iter().map(|n| n.len()).max().unwrap_or(1);
        let mut pos = from;
        let len = self.src.len();
        while pos < len {
            self.fill(pos, longest)?;
            let off = (pos - self.buf_start) as usize;
            let hay = &self.buf[off..];
            let best = needles
                .iter()
                .enumerate()
                .filter_map(|(i, n)| memmem::find(hay, n).map(|p| (p, i)))
                .min();
            if let Some((p, i)) = best {
                return Ok(Some((pos + p as u64, i)));
            }
            if self.buf_start + self.buf.len() as u64 >= len {
                break;
            }
            // Overlap so a needle spanning the chunk boundary is found.
            pos += (hay.len().saturating_sub(longest - 1)).max(1) as u64;
        }
        Ok(None)
    }

    pub fn find(&mut self, from: u64, needle: &[u8]) -> io::Result<Option<u64>> {
        Ok(self.find_any(from, &[needle])?.map(|(p, _)| p))
    }

    /// Bytes `[at - before, at + after)`, clamped to the source.
    pub fn around(&mut self, at: u64, before: usize, after: usize) -> io::Result<(u64, &[u8])> {
        let start = at.saturating_sub(before as u64);
        self.fill(start, (at - start) as usize + after)?;
        let off = (start - self.buf_start) as usize;
        let end = (off + (at - start) as usize + after).min(self.buf.len());
        Ok((start, &self.buf[off..end]))
    }
}

/// Last occurrence of `needle` within the final `window` bytes of `src`.
pub fn rfind_tail(src: &dyn ByteSource, needle: &[u8], window: usize) -> io::Result<Option<u64>> {
    let len = src.len();
    let start = len.saturating_sub(window as u64);
    let buf = read_vec(src, start, window)?;
    Ok(memmem::rfind(&buf, needle).map(|p| start + p as u64))
}
