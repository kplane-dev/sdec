//! Bit-level reader with bounded operations.

use crate::error::{BitError, BitResult};

/// A bit-level reader for decoding packed binary data.
///
/// All read operations are bounds-checked and return errors on failure.
/// The reader never panics on malformed input.
#[derive(Debug)]
pub struct BitReader<'a> {
    /// The underlying byte buffer.
    data: &'a [u8],
    /// Current bit position (0 = MSB of first byte).
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    /// Creates a new `BitReader` from a byte slice.
    #[must_use]
    pub const fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    /// Returns the number of bits remaining to read.
    #[must_use]
    pub const fn bits_remaining(&self) -> usize {
        self.data
            .len()
            .saturating_mul(8)
            .saturating_sub(self.bit_pos)
    }

    /// Returns `true` if there are no more bits to read.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bits_remaining() == 0
    }

    /// Returns the current bit position.
    #[must_use]
    pub const fn bit_position(&self) -> usize {
        self.bit_pos
    }

    /// Reads a single bit as a boolean.
    ///
    /// # Errors
    ///
    /// Returns [`BitError::EndOfBuffer`] if no bits remain.
    pub fn read_bool(&mut self) -> BitResult<bool> {
        if self.bits_remaining() == 0 {
            return Err(BitError::EndOfBuffer {
                requested: 1,
                available: 0,
            });
        }
        let byte_idx = self.bit_pos / 8;
        let bit_idx = self.bit_pos % 8;
        let bit = (self.data[byte_idx] >> (7 - bit_idx)) & 1;
        self.bit_pos += 1;
        Ok(bit == 1)
    }

    /// Reads up to 64 bits as an unsigned integer.
    ///
    /// # Errors
    ///
    /// Returns [`BitError::InvalidBitCount`] if `bits > 64`.
    /// Returns [`BitError::EndOfBuffer`] if insufficient bits remain.
    pub fn read_bits(&mut self, bits: usize) -> BitResult<u64> {
        if bits > 64 {
            return Err(BitError::InvalidBitCount { bits, max_bits: 64 });
        }
        if bits == 0 {
            return Ok(0);
        }
        if bits > self.bits_remaining() {
            return Err(BitError::EndOfBuffer {
                requested: bits,
                available: self.bits_remaining(),
            });
        }

        // Simple implementation; optimized bit slicing can be added later.
        let mut value = 0u64;
        for _ in 0..bits {
            value = (value << 1) | u64::from(self.read_bool()?);
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_reader() {
        let reader = BitReader::new(&[]);
        assert!(reader.is_empty());
        assert_eq!(reader.bits_remaining(), 0);
        assert_eq!(reader.bit_position(), 0);
    }

    #[test]
    fn read_from_empty_fails() {
        let mut reader = BitReader::new(&[]);
        let result = reader.read_bool();
        assert!(matches!(
            result,
            Err(BitError::EndOfBuffer {
                requested: 1,
                available: 0
            })
        ));
    }

    #[test]
    fn bits_remaining_single_byte() {
        let reader = BitReader::new(&[0xFF]);
        assert_eq!(reader.bits_remaining(), 8);
        assert!(!reader.is_empty());
    }

    #[test]
    fn bits_remaining_multiple_bytes() {
        let reader = BitReader::new(&[0x00, 0x00, 0x00]);
        assert_eq!(reader.bits_remaining(), 24);
    }

    #[test]
    fn read_bool_true() {
        let mut reader = BitReader::new(&[0b1000_0000]);
        assert!(reader.read_bool().unwrap());
        assert_eq!(reader.bit_position(), 1);
        assert_eq!(reader.bits_remaining(), 7);
    }

    #[test]
    fn read_bool_false() {
        let mut reader = BitReader::new(&[0b0000_0000]);
        assert!(!reader.read_bool().unwrap());
    }

    #[test]
    fn read_all_bits_in_byte() {
        let mut reader = BitReader::new(&[0b1010_0101]);
        assert!(reader.read_bool().unwrap()); // 1
        assert!(!reader.read_bool().unwrap()); // 0
        assert!(reader.read_bool().unwrap()); // 1
        assert!(!reader.read_bool().unwrap()); // 0
        assert!(!reader.read_bool().unwrap()); // 0
        assert!(reader.read_bool().unwrap()); // 1
        assert!(!reader.read_bool().unwrap()); // 0
        assert!(reader.read_bool().unwrap()); // 1
        assert!(reader.is_empty());
    }

    #[test]
    fn read_bits_zero() {
        let mut reader = BitReader::new(&[0xFF]);
        assert_eq!(reader.read_bits(0).unwrap(), 0);
        assert_eq!(reader.bit_position(), 0); // no bits consumed
    }

    #[test]
    fn read_bits_partial_byte() {
        let mut reader = BitReader::new(&[0b1010_1100]);
        // Read first 4 bits: 1010 = 10
        assert_eq!(reader.read_bits(4).unwrap(), 0b1010);
        assert_eq!(reader.bit_position(), 4);
        // Read next 4 bits: 1100 = 12
        assert_eq!(reader.read_bits(4).unwrap(), 0b1100);
        assert!(reader.is_empty());
    }

    #[test]
    fn read_bits_full_byte() {
        let mut reader = BitReader::new(&[0xAB]);
        assert_eq!(reader.read_bits(8).unwrap(), 0xAB);
        assert!(reader.is_empty());
    }

    #[test]
    fn read_bits_across_bytes() {
        let mut reader = BitReader::new(&[0b1111_0000, 0b0000_1111]);
        // Read 12 bits: 1111_0000_0000 = 0xF00
        assert_eq!(reader.read_bits(12).unwrap(), 0b1111_0000_0000);
        assert_eq!(reader.bits_remaining(), 4);
    }

    #[test]
    fn read_bits_too_many_requested() {
        let mut reader = BitReader::new(&[0xFF]);
        let result = reader.read_bits(16);
        assert!(matches!(
            result,
            Err(BitError::EndOfBuffer {
                requested: 16,
                available: 8
            })
        ));
    }

    #[test]
    fn read_bits_invalid_count() {
        let mut reader = BitReader::new(&[0xFF; 16]);
        let result = reader.read_bits(65);
        assert!(matches!(
            result,
            Err(BitError::InvalidBitCount {
                bits: 65,
                max_bits: 64
            })
        ));
    }

    #[test]
    fn read_bits_max_valid() {
        let data = [0xFF; 8];
        let mut reader = BitReader::new(&data);
        assert_eq!(reader.read_bits(64).unwrap(), u64::MAX);
    }

    #[test]
    fn reader_is_const_constructible() {
        const READER: BitReader<'static> = BitReader::new(&[1, 2, 3]);
        assert_eq!(READER.bits_remaining(), 24);
    }
}
