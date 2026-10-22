#![no_main]

use libfuzzer_sys::fuzz_target;
use pdfi_content::Issues;
use pdfi_content::parse::{OpReader, SourceData};

// Split the input in two streams (like a page's /Contents array) and read
// every operation, then read again from where the reader stood midway.
fuzz_target!(|data: &[u8]| {
    let cut = data.first().map_or(0, |&b| b as usize).min(data.len());
    let sources = [
        SourceData {
            id: None,
            data: data[..cut].to_vec(),
            error: None,
        },
        SourceData {
            id: None,
            data: data[cut..].to_vec(),
            error: None,
        },
    ];
    let mut issues = Issues::default();
    let mut r = OpReader::new(&sources);
    let mut mid = None;
    let mut n = 0;
    while r.next(&mut issues).is_some() {
        n += 1;
        if n == 3 {
            mid = Some(r.point());
        }
    }
    if let Some(p) = mid {
        let mut again = OpReader::at(&sources, &p);
        let mut rest = 0;
        while again.next(&mut Issues::default()).is_some() {
            rest += 1;
        }
        assert_eq!(rest + 3, n);
    }
});
