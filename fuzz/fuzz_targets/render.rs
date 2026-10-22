//! Render arbitrary bytes as the content of a page that has an image, a
//! form and a few color spaces to use, then hit-test it.
#![no_main]

use std::sync::Arc;

use libfuzzer_sys::fuzz_target;
use pdfi_content::{DocContent, Target};
use pdfi_model::DocModel;
use pdfi_render::{DocRender, Layer, Options, hit_test, render};
use pdfi_syntax::{MemSource, OpenOptions, PdfFile};

fn document(content: &[u8]) -> Vec<u8> {
    let objs: [Vec<u8>; 6] = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /XObject << /Im 5 0 R /Fm 6 0 R >> /ColorSpace << /I [/Indexed /DeviceRGB 1 <FF000000FF00>] /S [/Separation /X /DeviceGray << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >>] /L [/Lab << /WhitePoint [0.95 1 1.09] >>] >> >> >>".to_vec(),
        [
            format!("<< /Length {} >>\nstream\n", content.len()).as_bytes(),
            content,
            b"\nendstream",
        ]
        .concat(),
        b"<< /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 4 /Length 2 >>\nstream\n\x0f\xf0\nendstream".to_vec(),
        b"<< /Subtype /Form /BBox [0 0 10 10] /Length 18 >>\nstream\n0 0 20 20 re f /Im Do\nendstream".to_vec(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(b"xref\n0 7\n0000000000 65535 f\r\n");
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n\r\n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    out
}

fuzz_target!(|data: &[u8]| {
    let Ok(f) = PdfFile::open(Arc::new(MemSource::new(document(data))), OpenOptions::default())
    else {
        return;
    };
    let (dm, dc, dr) = (DocModel::new(), DocContent::new(), DocRender::new());
    let c = dc.on(dm.on(&f));
    let Ok(a) = c.analyze(Target::Page(0)) else { return };
    let o = Options {
        scale: 0.5,
        tile: None,
        until: None,
        layer: Layer::Content,
    };
    let _ = render(&dr, &c, &a, &o, &|| false);
    let _ = hit_test(&dr, &c, &a, (25.0, 25.0), 2.0, None, &|| false);
});
