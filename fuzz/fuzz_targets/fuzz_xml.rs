#![no_main]

#[macro_use] extern crate libfuzzer_sys;
extern crate xmlparser;

use std::str;

use xmlparser::FromSpan;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = str::from_utf8(data) {
        let mut n = 0;
        for _ in xmlparser::Tokenizer::from_str(text) {
            n += 1;

            if n == 1000 {
                panic!("endless loop");
            }
        }
    }
});