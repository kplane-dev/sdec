#![no_main]

use bitstream::BitReader;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut reader = BitReader::new(data);
    let mut idx = 0usize;

    // Use input bytes to drive a bounded sequence of operations.
    while idx < data.len() && idx < 1024 {
        let op = data[idx] % 6;
        idx += 1;

        match op {
            0 => {
                let _ = reader.read_bit();
            }
            1 => {
                let bits = (data[idx.saturating_sub(1)] % 64).saturating_add(1);
                let _ = reader.read_bits(bits);
            }
            2 => {
                let _ = reader.align_to_byte();
            }
            3 => {
                let _ = reader.read_u32_aligned();
            }
            4 => {
                let _ = reader.read_varu32();
            }
            _ => {
                let _ = reader.read_vars32();
            }
        }
    }
});
