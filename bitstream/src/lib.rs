//! Low-level bit packing primitives for the sdec codec.
//!
//! This crate provides [`BitWriter`] and [`BitReader`] for bit-level encoding and decoding.
//! It is designed for bounded, panic-free operation with explicit error handling.
//!
//! # Design Principles
//!
//! - **No unsafe code** - Safety is paramount.
//! - **Bounded operations** - All reads/writes are bounds-checked.
//! - **No domain knowledge** - This crate knows nothing about entities, components, or game state.
//! - **Explicit errors** - All failures return structured errors, never panic.
//!
//! # Example
//!
//! ```
//! use bitstream::{BitWriter, BitReader};
//!
//! let mut writer = BitWriter::new();
//! writer.write_bool(true);
//! writer.write_bits(42, 7).unwrap();
//!
//! let bytes = writer.finish();
//!
//! let mut reader = BitReader::new(&bytes);
//! assert_eq!(reader.read_bool().unwrap(), true);
//! assert_eq!(reader.read_bits(7).unwrap(), 42);
//! ```

mod error;
mod reader;
mod writer;

pub use error::{BitError, BitResult};
pub use reader::BitReader;
pub use writer::BitWriter;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_roundtrip() {
        let writer = BitWriter::new();
        let bytes = writer.finish();
        assert!(bytes.is_empty());

        let reader = BitReader::new(&bytes);
        assert!(reader.is_empty());
    }

    #[test]
    fn single_bool_roundtrip() {
        let mut writer = BitWriter::new();
        writer.write_bool(true);
        let bytes = writer.finish();

        let mut reader = BitReader::new(&bytes);
        assert!(reader.read_bool().unwrap());
    }

    #[test]
    fn multiple_bools_roundtrip() {
        let mut writer = BitWriter::new();
        writer.write_bool(true);
        writer.write_bool(false);
        writer.write_bool(true);
        writer.write_bool(true);
        writer.write_bool(false);
        let bytes = writer.finish();

        let mut reader = BitReader::new(&bytes);
        assert!(reader.read_bool().unwrap());
        assert!(!reader.read_bool().unwrap());
        assert!(reader.read_bool().unwrap());
        assert!(reader.read_bool().unwrap());
        assert!(!reader.read_bool().unwrap());
    }

    #[test]
    fn bits_roundtrip_various_sizes() {
        let test_cases = [
            (0b1010u64, 4),
            (0xFFu64, 8),
            (0xABCDu64, 16),
            (0x1234_5678u64, 32),
            (u64::MAX, 64),
        ];

        for (value, bits) in test_cases {
            let mut writer = BitWriter::new();
            writer.write_bits(value, bits).unwrap();
            let bytes = writer.finish();

            let mut reader = BitReader::new(&bytes);
            let read_value = reader.read_bits(bits).unwrap();
            assert_eq!(
                read_value, value,
                "roundtrip failed for {bits}-bit value {value}"
            );
        }
    }

    #[test]
    fn mixed_roundtrip() {
        let mut writer = BitWriter::new();
        writer.write_bool(true);
        writer.write_bits(0b1010, 4).unwrap();
        writer.write_bool(false);
        writer.write_bits(0xFF, 8).unwrap();
        writer.write_bits(42, 7).unwrap();
        let bytes = writer.finish();

        let mut reader = BitReader::new(&bytes);
        assert!(reader.read_bool().unwrap());
        assert_eq!(reader.read_bits(4).unwrap(), 0b1010);
        assert!(!reader.read_bool().unwrap());
        assert_eq!(reader.read_bits(8).unwrap(), 0xFF);
        assert_eq!(reader.read_bits(7).unwrap(), 42);
    }

    #[test]
    fn doctest_example() {
        let mut writer = BitWriter::new();
        writer.write_bool(true);
        writer.write_bits(42, 7).unwrap();

        let bytes = writer.finish();

        let mut reader = BitReader::new(&bytes);
        assert!(reader.read_bool().unwrap());
        assert_eq!(reader.read_bits(7).unwrap(), 42);
    }
}
