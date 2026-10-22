//! Parse arbitrary bytes as a sequence of objects.
#![no_main]

use libfuzzer_sys::fuzz_target;
use pdfi_syntax::parser::Parser;

fuzz_target!(|data: &[u8]| {
    let mut p = Parser::new(data, 0, true);
    for _ in 0..1000 {
        if p.parse_object().is_err() {
            break;
        }
    }
});
