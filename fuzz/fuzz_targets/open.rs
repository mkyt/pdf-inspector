//! Open arbitrary bytes as a PDF, load every object and decode every stream.
#![no_main]

use std::io::Read;
use std::sync::Arc;

use libfuzzer_sys::fuzz_target;
use pdfi_syntax::{MemSource, Object, OpenOptions, PdfFile};

fuzz_target!(|data: &[u8]| {
    let Ok(f) = PdfFile::open(Arc::new(MemSource::new(data.to_vec())), OpenOptions::default()) else {
        return;
    };
    let ids: Vec<_> = f.xref().live_ids().take(10_000).collect();
    for id in ids {
        if let Ok(o) = f.get(id) {
            if let Object::Stream(s) = &o.node.obj {
                let (r, _) = f.decoded_reader(id, s);
                let _ = r.take(1 << 20).read_to_end(&mut Vec::new());
            }
        }
    }
});
