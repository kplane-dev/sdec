//! Bit-level writer for encoding packed binary data.

use crate::error::{BitError, BitResult};

/// A bit-level writer for encoding packed binary data.
///
/// Writes are accumulated in an internal buffer. Call [`finish`](Self::finish)
/// to get the final byte buffer.
#[derive(Debug, Default)]
pub struct BitWriter {
    /// The accumulated bytes.
    bytes: Vec<u8>,
    /// Current byte being written (not yet pushed to bytes).
    current_byte: u8,
    /// Number of bits written to `current_byte` (0-7).
    bit_count: u8,
}

impl BitWriter {
    /// Creates a new empty `BitWriter`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new `BitWriter` with pre-allocated capacity.
    #[must_use]
    pub fn with_capacity(bytes: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(bytes),
            current_byte: 0,
            bit_count: 0,
        }
    }

    /// Returns the number of bits written so far.
    #[must_use]
    pub fn bits_written(&self) -> usize {
        self.bytes.len() * 8 + self.bit_count as usize
    }

    /// Writes a single bit.
    pub fn write_bool(&mut self, value: bool) {
        self.current_byte = (self.current_byte << 1) | u8::from(value);
        self.bit_count += 1;
        if self.bit_count == 8 {
            self.bytes.push(self.current_byte);
            self.current_byte = 0;
            self.bit_count = 0;
        }
    }

    /// Writes up to 64 bits from an unsigned integer.
    ///
    /// # Errors
    ///
    /// Returns [`BitError::InvalidBitCount`] if `bits > 64`.
    /// Returns [`BitError::ValueOutOfRange`] if `value` doesn't fit in `bits`.
    pub fn write_bits(&mut self, value: u64, bits: usize) -> BitResult<()> {
        if bits > 64 {
            return Err(BitError::InvalidBitCount { bits, max_bits: 64 });
        }
        if bits == 0 {
            return Ok(());
        }
        if bits < 64 && value >= (1u64 << bits) {
            return Err(BitError::ValueOutOfRange { value, bits });
        }

        // Simple implementation; optimized bit slicing can be added later.
        for i in (0..bits).rev() {
            self.write_bool((value >> i) & 1 == 1);
        }
        Ok(())
    }

    /// Finishes writing and returns the byte buffer.
    ///
    /// If the last byte is incomplete, it is padded with zeros on the right.
    #[must_use]
    pub fn finish(mut self) -> Vec<u8> {
        if self.bit_count > 0 {
            // Pad the remaining bits with zeros
            self.current_byte <<= 8 - self.bit_count;
            self.bytes.push(self.current_byte);
        }
        self.bytes
    }

    /// Finishes writing and appends to the provided buffer.
    ///
    /// If the last byte is incomplete, it is padded with zeros on the right.
    pub fn finish_into(mut self, buf: &mut Vec<u8>) {
        if self.bit_count > 0 {
            self.current_byte <<= 8 - self.bit_count;
            self.bytes.push(self.current_byte);
        }
        buf.append(&mut self.bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_writer() {
        let writer = BitWriter::new();
        assert_eq!(writer.bits_written(), 0);
        let bytes = writer.finish();
        assert!(bytes.is_empty());
    }

    #[test]
    fn write_single_bit_true() {
        let mut writer = BitWriter::new();
        writer.write_bool(true);
        assert_eq!(writer.bits_written(), 1);
        let bytes = writer.finish();
        // Single bit 1, padded with 7 zeros = 0b1000_0000
        assert_eq!(bytes, vec![0b1000_0000]);
    }

    #[test]
    fn write_single_bit_false() {
        let mut writer = BitWriter::new();
        writer.write_bool(false);
        let bytes = writer.finish();
        assert_eq!(bytes, vec![0b0000_0000]);
    }

    #[test]
    fn write_full_byte() {
        let mut writer = BitWriter::new();
        for bit in [true, false, true, false, true, false, true, false] {
            writer.write_bool(bit);
        }
        assert_eq!(writer.bits_written(), 8);
        let bytes = writer.finish();
        assert_eq!(bytes, vec![0b1010_1010]);
    }

    #[test]
    fn write_partial_byte_with_padding() {
        let mut writer = BitWriter::new();
        // Write 5 bits: 11010
        writer.write_bool(true);
        writer.write_bool(true);
        writer.write_bool(false);
        writer.write_bool(true);
        writer.write_bool(false);
        let bytes = writer.finish();
        // 11010 + 000 padding = 0b1101_0000
        assert_eq!(bytes, vec![0b1101_0000]);
    }

    #[test]
    fn write_bits_zero() {
        let mut writer = BitWriter::new();
        writer.write_bits(0xFF, 0).unwrap();
        assert_eq!(writer.bits_written(), 0);
        let bytes = writer.finish();
        assert!(bytes.is_empty());
    }

    #[test]
    fn write_bits_partial() {
        let mut writer = BitWriter::new();
        writer.write_bits(0b1010, 4).unwrap();
        assert_eq!(writer.bits_written(), 4);
        let bytes = writer.finish();
        assert_eq!(bytes, vec![0b1010_0000]);
    }

    #[test]
    fn write_bits_full_byte() {
        let mut writer = BitWriter::new();
        writer.write_bits(0xAB, 8).unwrap();
        let bytes = writer.finish();
        assert_eq!(bytes, vec![0xAB]);
    }

    #[test]
    fn write_bits_multiple_bytes() {
        let mut writer = BitWriter::new();
        writer.write_bits(0xABCD, 16).unwrap();
        let bytes = writer.finish();
        assert_eq!(bytes, vec![0xAB, 0xCD]);
    }

    #[test]
    fn write_bits_across_byte_boundary() {
        let mut writer = BitWriter::new();
        writer.write_bits(0b1111, 4).unwrap();
        writer.write_bits(0b1010_1010, 8).unwrap();
        let bytes = writer.finish();
        // 1111 + 10101010 = 1111_1010 1010_0000
        assert_eq!(bytes, vec![0b1111_1010, 0b1010_0000]);
    }

    #[test]
    fn write_bits_invalid_count() {
        let mut writer = BitWriter::new();
        let result = writer.write_bits(0, 65);
        assert!(matches!(
            result,
            Err(BitError::InvalidBitCount {
                bits: 65,
                max_bits: 64
            })
        ));
    }

    #[test]
    fn write_bits_value_out_of_range() {
        let mut writer = BitWriter::new();
        // Try to write 256 in 8 bits (max is 255)
        let result = writer.write_bits(256, 8);
        assert!(matches!(
            result,
            Err(BitError::ValueOutOfRange {
                value: 256,
                bits: 8
            })
        ));
    }

    #[test]
    fn write_bits_max_value_fits() {
        let mut writer = BitWriter::new();
        // 255 fits in 8 bits
        writer.write_bits(255, 8).unwrap();
        let bytes = writer.finish();
        assert_eq!(bytes, vec![0xFF]);
    }

    #[test]
    fn write_bits_64_bits() {
        let mut writer = BitWriter::new();
        writer.write_bits(u64::MAX, 64).unwrap();
        let bytes = writer.finish();
        assert_eq!(bytes, vec![0xFF; 8]);
    }

    #[test]
    fn with_capacity() {
        let writer = BitWriter::with_capacity(100);
        assert_eq!(writer.bits_written(), 0);
        // Just verify it doesn't panic
    }

    #[test]
    fn finish_into() {
        let mut writer = BitWriter::new();
        writer.write_bits(0xAB, 8).unwrap();

        let mut buf = vec![0x00, 0x11];
        writer.finish_into(&mut buf);
        assert_eq!(buf, vec![0x00, 0x11, 0xAB]);
    }

    #[test]
    fn finish_into_with_padding() {
        let mut writer = BitWriter::new();
        writer.write_bool(true);

        let mut buf = Vec::new();
        writer.finish_into(&mut buf);
        assert_eq!(buf, vec![0b1000_0000]);
    }

    #[test]
    fn writer_default() {
        let writer = BitWriter::default();
        assert_eq!(writer.bits_written(), 0);
    }
}
