//! Cheap deterministic fuzzing: corrupt fixtures and make sure nothing panics.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use pdfi_syntax::{MemSource, Object, OpenOptions, PdfFile};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn exercise(data: Vec<u8>) {
    let opts = OpenOptions {
        password: Some(b"user".to_vec()),
        ..Default::default()
    };
    let Ok(f) = PdfFile::open(Arc::new(MemSource::new(data)), opts) else {
        return;
    };
    let ids: Vec<_> = f.xref().live_ids().take(200).collect();
    for id in ids {
        if let Ok(o) = f.get(id) {
            if let Object::Stream(s) = &o.node.obj {
                let (r, _) = f.decoded_reader(id, s);
                let _ = r.take(1 << 20).read_to_end(&mut Vec::new());
            }
        }
    }
    let _ = pdfi_syntax::write::rewrite(&f, std::io::sink(), &Default::default());
}

#[test]
fn corrupted_fixtures_do_not_panic() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    for name in [
        "base.pdf",
        "objstm.pdf",
        "incremental.pdf",
        "aes-256-objstm.pdf",
        "rc4-128.pdf",
        "linearized.pdf",
    ] {
        let orig = std::fs::read(dir.join(name)).unwrap();
        for _ in 0..300 {
            let mut d = orig.clone();
            match rng.below(4) {
                0 => {
                    for _ in 0..1 + rng.below(8) {
                        let i = rng.below(d.len());
                        d[i] = rng.next() as u8;
                    }
                }
                1 => d.truncate(rng.below(d.len())),
                2 => {
                    let i = rng.below(d.len());
                    let n = rng.below(64).min(d.len() - i);
                    d.drain(i..i + n);
                }
                _ => {
                    let i = rng.below(d.len());
                    let tok: &[u8] = [
                        b"<<".as_slice(),
                        b"[",
                        b"(",
                        b"0 0 R",
                        b"endobj",
                        b"stream\n",
                        b"999999999999 ",
                    ][rng.below(7)];
                    d.splice(i..i, tok.iter().copied());
                }
            }
            exercise(d);
        }
    }
}
