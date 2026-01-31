//! Bit-level writers for encoding packed binary data.

use crate::error::{BitError, BitResult};

/// A bounded bit-level writer for encoding packed binary data.
///
/// The writer never grows the buffer. Writes past the end return
/// [`BitError::WriteOverflow`].
#[derive(Debug)]
pub struct BitWriter<'a> {
    buf: &'a mut [u8],
    bit_pos: usize,
}

impl<'a> BitWriter<'a> {
    /// Creates a new `BitWriter` over a fixed buffer.
    #[must_use]
    pub const fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, bit_pos: 0 }
    }

    /// Returns the number of bits written so far.
    #[must_use]
    pub const fn bits_written(&self) -> usize {
        self.bit_pos
    }

    /// Returns the number of bits remaining in the buffer.
    #[must_use]
    pub const fn bits_remaining(&self) -> usize {
        self.buf
            .len()
            .saturating_mul(8)
            .saturating_sub(self.bit_pos)
    }

    /// Writes a single bit.
    pub fn write_bit(&mut self, value: bool) -> BitResult<()> {
        self.ensure_bits(1)?;
        self.write_bit_unchecked(value);
        Ok(())
    }

    /// Writes up to 64 bits from an unsigned integer.
    ///
    /// # Errors
    ///
    /// Returns [`BitError::InvalidBitCount`] if `bits > 64`.
    /// Returns [`BitError::ValueOutOfRange`] if `value` doesn't fit in `bits`.
    pub fn write_bits(&mut self, value: u64, bits: u8) -> BitResult<()> {
        if bits > 64 {
            return Err(BitError::InvalidBitCount { bits, max_bits: 64 });
        }
        if bits == 0 {
            return Ok(());
        }
        if bits < 64 && value >= (1u64 << bits) {
            return Err(BitError::ValueOutOfRange { value, bits });
        }
        self.ensure_bits(bits as usize)?;
        for i in (0..bits).rev() {
            let bit = (value >> i) & 1 == 1;
            self.write_bit_unchecked(bit);
        }
        Ok(())
    }

    /// Pads with zero bits until the next byte boundary.
    pub fn align_to_byte(&mut self) -> BitResult<()> {
        let rem = self.bit_pos % 8;
        if rem == 0 {
            return Ok(());
        }
        let padding = 8 - rem;
        self.ensure_bits(padding)?;
        for _ in 0..padding {
            self.write_bit_unchecked(false);
        }
        Ok(())
    }

    /// Writes a byte-aligned `u8`.
    pub fn write_u8_aligned(&mut self, value: u8) -> BitResult<()> {
        self.ensure_aligned()?;
        self.ensure_bits(8)?;
        let idx = self.bit_pos / 8;
        self.buf[idx] = value;
        self.bit_pos += 8;
        Ok(())
    }

    /// Writes a byte-aligned `u16` (little-endian).
    pub fn write_u16_aligned(&mut self, value: u16) -> BitResult<()> {
        self.write_bytes_aligned(&value.to_le_bytes())
    }

    /// Writes a byte-aligned `u32` (little-endian).
    pub fn write_u32_aligned(&mut self, value: u32) -> BitResult<()> {
        self.write_bytes_aligned(&value.to_le_bytes())
    }

    /// Writes a byte-aligned `u64` (little-endian).
    pub fn write_u64_aligned(&mut self, value: u64) -> BitResult<()> {
        self.write_bytes_aligned(&value.to_le_bytes())
    }

    /// Writes a byte-aligned varint `u32`.
    pub fn write_varu32(&mut self, mut value: u32) -> BitResult<()> {
        self.ensure_aligned()?;
        loop {
            let mut byte = (value & 0x7F) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            self.write_u8_aligned(byte)?;
            if value == 0 {
                break;
            }
        }
        Ok(())
    }

    /// Writes a byte-aligned zigzag varint `i32`.
    pub fn write_vars32(&mut self, value: i32) -> BitResult<()> {
        let zigzag = ((value << 1) ^ (value >> 31)) as u32;
        self.write_varu32(zigzag)
    }

    /// Finishes writing and returns the number of bytes used.
    #[must_use]
    pub fn finish(self) -> usize {
        self.bit_pos.div_ceil(8)
    }

    fn ensure_bits(&self, bits: usize) -> BitResult<()> {
        let available = self.bits_remaining();
        if bits > available {
            return Err(BitError::WriteOverflow {
                attempted: bits,
                available,
            });
        }
        Ok(())
    }

    fn ensure_aligned(&self) -> BitResult<()> {
        if self.bit_pos % 8 != 0 {
            return Err(BitError::MisalignedAccess {
                bit_position: self.bit_pos,
            });
        }
        Ok(())
    }

    fn write_bytes_aligned(&mut self, bytes: &[u8]) -> BitResult<()> {
        self.ensure_aligned()?;
        self.ensure_bits(bytes.len() * 8)?;
        let idx = self.bit_pos / 8;
        self.buf[idx..idx + bytes.len()].copy_from_slice(bytes);
        self.bit_pos += bytes.len() * 8;
        Ok(())
    }

    fn write_bit_unchecked(&mut self, value: bool) {
        let byte_idx = self.bit_pos / 8;
        let bit_idx = self.bit_pos % 8;
        let mask = 1u8 << (7 - bit_idx);
        if value {
            self.buf[byte_idx] |= mask;
        } else {
            self.buf[byte_idx] &= !mask;
        }
        self.bit_pos += 1;
    }
}

/// A growable bit-level writer backed by a `Vec<u8>`.
#[derive(Debug, Default)]
pub struct BitVecWriter {
    buf: Vec<u8>,
    bit_pos: usize,
}

impl BitVecWriter {
    /// Creates a new empty `BitVecWriter`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a new `BitVecWriter` with pre-allocated capacity.
    #[must_use]
    pub fn with_capacity(bytes: usize) -> Self {
        Self {
            buf: Vec::with_capacity(bytes),
            bit_pos: 0,
        }
    }

    /// Returns the number of bits written so far.
    #[must_use]
    pub const fn bits_written(&self) -> usize {
        self.bit_pos
    }

    /// Writes a single bit.
    pub fn write_bit(&mut self, value: bool) {
        self.ensure_capacity_bits(1);
        self.write_bit_unchecked(value);
    }

    /// Writes up to 64 bits from an unsigned integer.
    pub fn write_bits(&mut self, value: u64, bits: u8) -> BitResult<()> {
        if bits > 64 {
            return Err(BitError::InvalidBitCount { bits, max_bits: 64 });
        }
        if bits == 0 {
            return Ok(());
        }
        if bits < 64 && value >= (1u64 << bits) {
            return Err(BitError::ValueOutOfRange { value, bits });
        }
        self.ensure_capacity_bits(bits as usize);
        for i in (0..bits).rev() {
            let bit = (value >> i) & 1 == 1;
            self.write_bit_unchecked(bit);
        }
        Ok(())
    }

    /// Pads with zero bits until the next byte boundary.
    pub fn align_to_byte(&mut self) {
        let rem = self.bit_pos % 8;
        if rem == 0 {
            return;
        }
        let padding = 8 - rem;
        self.ensure_capacity_bits(padding);
        for _ in 0..padding {
            self.write_bit_unchecked(false);
        }
    }

    /// Writes a byte-aligned `u8`.
    pub fn write_u8_aligned(&mut self, value: u8) -> BitResult<()> {
        self.ensure_aligned()?;
        self.ensure_capacity_bits(8);
        let idx = self.bit_pos / 8;
        self.buf[idx] = value;
        self.bit_pos += 8;
        Ok(())
    }

    /// Writes a byte-aligned `u16` (little-endian).
    pub fn write_u16_aligned(&mut self, value: u16) -> BitResult<()> {
        self.write_bytes_aligned(&value.to_le_bytes())
    }

    /// Writes a byte-aligned `u32` (little-endian).
    pub fn write_u32_aligned(&mut self, value: u32) -> BitResult<()> {
        self.write_bytes_aligned(&value.to_le_bytes())
    }

    /// Writes a byte-aligned `u64` (little-endian).
    pub fn write_u64_aligned(&mut self, value: u64) -> BitResult<()> {
        self.write_bytes_aligned(&value.to_le_bytes())
    }

    /// Writes a byte-aligned varint `u32`.
    pub fn write_varu32(&mut self, mut value: u32) -> BitResult<()> {
        self.ensure_aligned()?;
        loop {
            let mut byte = (value & 0x7F) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            self.write_u8_aligned(byte)?;
            if value == 0 {
                break;
            }
        }
        Ok(())
    }

    /// Writes a byte-aligned zigzag varint `i32`.
    pub fn write_vars32(&mut self, value: i32) -> BitResult<()> {
        let zigzag = ((value << 1) ^ (value >> 31)) as u32;
        self.write_varu32(zigzag)
    }

    /// Finishes writing and returns the byte buffer.
    #[must_use]
    pub fn finish(mut self) -> Vec<u8> {
        let bytes = self.bit_pos.div_ceil(8);
        self.buf.truncate(bytes);
        self.buf
    }

    fn ensure_capacity_bits(&mut self, bits: usize) {
        let required_bits = self.bit_pos + bits;
        let required_bytes = required_bits.div_ceil(8);
        if required_bytes > self.buf.len() {
            self.buf.resize(required_bytes, 0);
        }
    }

    fn ensure_aligned(&self) -> BitResult<()> {
        if self.bit_pos % 8 != 0 {
            return Err(BitError::MisalignedAccess {
                bit_position: self.bit_pos,
            });
        }
        Ok(())
    }

    fn write_bytes_aligned(&mut self, bytes: &[u8]) -> BitResult<()> {
        self.ensure_aligned()?;
        self.ensure_capacity_bits(bytes.len() * 8);
        let idx = self.bit_pos / 8;
        self.buf[idx..idx + bytes.len()].copy_from_slice(bytes);
        self.bit_pos += bytes.len() * 8;
        Ok(())
    }

    fn write_bit_unchecked(&mut self, value: bool) {
        let byte_idx = self.bit_pos / 8;
        let bit_idx = self.bit_pos % 8;
        let mask = 1u8 << (7 - bit_idx);
        if value {
            self.buf[byte_idx] |= mask;
        } else {
            self.buf[byte_idx] &= !mask;
        }
        self.bit_pos += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_overflow() {
        let mut buf = [0u8; 1];
        let mut writer = BitWriter::new(&mut buf);
        writer.write_bits(0xFF, 8).unwrap();
        let err = writer.write_bit(true).unwrap_err();
        assert!(matches!(err, BitError::WriteOverflow { .. }));
    }

    #[test]
    fn bounded_write_and_finish() {
        let mut buf = [0u8; 2];
        let mut writer = BitWriter::new(&mut buf);
        writer.write_bits(0b1010, 4).unwrap();
        writer.align_to_byte().unwrap();
        writer.write_u8_aligned(0xAB).unwrap();
        let bytes = writer.finish();
        assert_eq!(bytes, 2);
        assert_eq!(&buf[..2], &[0b1010_0000, 0xAB]);
    }

    #[test]
    fn vec_writer_roundtrip_bits() {
        let mut writer = BitVecWriter::new();
        writer.write_bits(0b1010, 4).unwrap();
        writer.write_bits(0xAB, 8).unwrap();
        let bytes = writer.finish();
        assert_eq!(bytes, vec![0b1010_1010, 0b1011_0000]);
    }

    #[test]
    fn vec_writer_align() {
        let mut writer = BitVecWriter::new();
        writer.write_bits(0b1010, 4).unwrap();
        writer.align_to_byte();
        writer.write_u8_aligned(0xFF).unwrap();
        let bytes = writer.finish();
        assert_eq!(bytes, vec![0b1010_0000, 0xFF]);
    }

    #[test]
    fn vec_writer_varint() {
        let mut writer = BitVecWriter::new();
        writer.align_to_byte();
        writer.write_varu32(300).unwrap();
        let bytes = writer.finish();
        assert_eq!(bytes, vec![0xAC, 0x02]);
    }

    #[test]
    fn vec_writer_zigzag() {
        let mut writer = BitVecWriter::new();
        writer.align_to_byte();
        writer.write_vars32(-1).unwrap();
        let bytes = writer.finish();
        assert_eq!(bytes, vec![0x01]);
    }
}
