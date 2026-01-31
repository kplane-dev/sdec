//! Bit-level reader with bounded operations.

use crate::error::{BitError, BitResult};

/// A bit-level reader for decoding packed binary data.
///
/// All read operations are bounds-checked and return errors on failure.
/// The reader never panics on malformed input.
#[derive(Debug)]
pub struct BitReader<'a> {
    data: &'a [u8],
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
    pub fn read_bit(&mut self) -> BitResult<bool> {
        if self.bits_remaining() == 0 {
            return Err(BitError::UnexpectedEof {
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
    pub fn read_bits(&mut self, bits: u8) -> BitResult<u64> {
        if bits > 64 {
            return Err(BitError::InvalidBitCount { bits, max_bits: 64 });
        }
        if bits == 0 {
            return Ok(0);
        }
        if bits as usize > self.bits_remaining() {
            return Err(BitError::UnexpectedEof {
                requested: bits as usize,
                available: self.bits_remaining(),
            });
        }

        let mut value = 0u64;
        for _ in 0..bits {
            value = (value << 1) | u64::from(self.read_bit()?);
        }
        Ok(value)
    }

    /// Aligns to the next byte boundary.
    pub fn align_to_byte(&mut self) -> BitResult<()> {
        let rem = self.bit_pos % 8;
        if rem == 0 {
            return Ok(());
        }
        let skip = 8 - rem;
        if skip > self.bits_remaining() {
            return Err(BitError::UnexpectedEof {
                requested: skip,
                available: self.bits_remaining(),
            });
        }
        self.bit_pos += skip;
        Ok(())
    }

    /// Reads a byte-aligned `u8`.
    pub fn read_u8_aligned(&mut self) -> BitResult<u8> {
        self.ensure_aligned()?;
        self.ensure_bits(8)?;
        let idx = self.bit_pos / 8;
        let value = self.data[idx];
        self.bit_pos += 8;
        Ok(value)
    }

    /// Reads a byte-aligned `u16` (little-endian).
    pub fn read_u16_aligned(&mut self) -> BitResult<u16> {
        let bytes = self.read_aligned_bytes::<2>()?;
        Ok(u16::from_le_bytes(bytes))
    }

    /// Reads a byte-aligned `u32` (little-endian).
    pub fn read_u32_aligned(&mut self) -> BitResult<u32> {
        let bytes = self.read_aligned_bytes::<4>()?;
        Ok(u32::from_le_bytes(bytes))
    }

    /// Reads a byte-aligned `u64` (little-endian).
    pub fn read_u64_aligned(&mut self) -> BitResult<u64> {
        let bytes = self.read_aligned_bytes::<8>()?;
        Ok(u64::from_le_bytes(bytes))
    }

    /// Reads a byte-aligned varint `u32`.
    pub fn read_varu32(&mut self) -> BitResult<u32> {
        self.ensure_aligned()?;
        let mut result = 0u32;
        for shift in (0..35).step_by(7) {
            let byte = self.read_u8_aligned()?;
            result |= u32::from(byte & 0x7F) << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
        }
        Err(BitError::InvalidVarint)
    }

    /// Reads a byte-aligned zigzag varint `i32`.
    pub fn read_vars32(&mut self) -> BitResult<i32> {
        let value = self.read_varu32()?;
        let decoded = ((value >> 1) as i32) ^ (-((value & 1) as i32));
        Ok(decoded)
    }

    fn ensure_aligned(&self) -> BitResult<()> {
        if self.bit_pos % 8 != 0 {
            return Err(BitError::MisalignedAccess {
                bit_position: self.bit_pos,
            });
        }
        Ok(())
    }

    fn ensure_bits(&self, bits: usize) -> BitResult<()> {
        let available = self.bits_remaining();
        if bits > available {
            return Err(BitError::UnexpectedEof {
                requested: bits,
                available,
            });
        }
        Ok(())
    }

    fn read_aligned_bytes<const N: usize>(&mut self) -> BitResult<[u8; N]> {
        self.ensure_aligned()?;
        self.ensure_bits(N * 8)?;
        let idx = self.bit_pos / 8;
        let mut out = [0u8; N];
        out.copy_from_slice(&self.data[idx..idx + N]);
        self.bit_pos += N * 8;
        Ok(out)
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
        let result = reader.read_bit();
        assert!(matches!(result, Err(BitError::UnexpectedEof { .. })));
    }

    #[test]
    fn read_bits_across_bytes() {
        let mut reader = BitReader::new(&[0b1111_0000, 0b0000_1111]);
        assert_eq!(reader.read_bits(12).unwrap(), 0b1111_0000_0000);
        assert_eq!(reader.bits_remaining(), 4);
    }

    #[test]
    fn read_aligned_u32() {
        let mut reader = BitReader::new(&[0x78, 0x56, 0x34, 0x12]);
        assert_eq!(reader.read_u32_aligned().unwrap(), 0x1234_5678);
    }

    #[test]
    fn read_misaligned_fails() {
        let mut reader = BitReader::new(&[0xFF, 0xFF]);
        reader.read_bits(1).unwrap();
        let err = reader.read_u8_aligned().unwrap_err();
        assert!(matches!(err, BitError::MisalignedAccess { .. }));
    }

    #[test]
    fn read_varu32() {
        let mut reader = BitReader::new(&[0xAC, 0x02]);
        assert_eq!(reader.read_varu32().unwrap(), 300);
    }

    #[test]
    fn read_vars32() {
        let mut reader = BitReader::new(&[0x01]);
        assert_eq!(reader.read_vars32().unwrap(), -1);
    }

    #[test]
    fn read_varu32_invalid() {
        let mut reader = BitReader::new(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01]);
        let err = reader.read_varu32().unwrap_err();
        assert!(matches!(err, BitError::InvalidVarint));
    }
}
