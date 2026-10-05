extern crate alloc;

#[allow(dead_code, unused_imports)]
#[path = "../src/name.rs"]
mod name;

use std::{hint::black_box, time::Instant};

fn crc_reference(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &byte in bytes {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ 0x1021
            };
        }
    }
    crc
}

fn row(label: &str, bytes: usize, samples: usize, mut operation: impl FnMut() -> usize) {
    let iterations = (1_000_000 / bytes).max(1000);
    let mut times = Vec::new();
    for sample in 0..=samples {
        let start = Instant::now();
        for _ in 0..iterations {
            black_box(operation());
        }
        if sample > 0 {
            times.push(start.elapsed().as_nanos());
        }
    }
    times.sort_unstable();
    let median = times[times.len() / 2];
    let per_op = median as f64 / iterations as f64;
    println!(
        "{label},{bytes},{iterations},{median},{per_op:.3},{:.3}",
        per_op / bytes as f64
    );
}

fn main() {
    let samples: usize =
        std::env::var("HADRIS_UDF_BENCH_SAMPLES").map_or(7, |s| s.parse().unwrap());
    assert!(samples > 0);
    println!("kernel,input_bytes,iterations,median_ns,ns_per_op,ns_per_byte");
    for len in [22, 52, 168, 1024, 2048, 4096] {
        let bytes: Vec<u8> = (0..len).map(|i| (i * 31 % 251) as u8).collect();
        let expected = crc_reference(&bytes);
        assert_eq!(hadris_udf::raw::crc16(&bytes), expected);
        for split in [0, len / 2, len] {
            let crc = hadris_udf::raw::crc16(&bytes[..split]);
            assert_eq!(
                hadris_udf::raw::crc16_update(crc, &bytes[split..]),
                expected
            );
        }
        row(&format!("crc/{len}"), len, samples, || {
            hadris_udf::raw::crc16(black_box(&bytes)) as usize
        });
    }
    for (label, text) in [
        ("ascii-short", "entry-0999".to_string()),
        ("ascii-long", "a".repeat(240)),
        ("latin1", "café".repeat(50)),
        ("utf16-short", "λ-file".to_string()),
        ("utf16-long", "λ".repeat(120)),
        ("utf16-surrogates", "\u{1F600}".repeat(60)),
    ] {
        let raw = name::encode_cs0(&text);
        let mut out = [0; 1024];
        let len = name::decode_name(&raw, &mut out).unwrap();
        assert_eq!(&out[..len], text.as_bytes());
        row(&format!("name/{label}"), raw.len(), samples, || {
            name::decode_name(black_box(&raw), black_box(&mut out)).unwrap()
        });
    }
}
