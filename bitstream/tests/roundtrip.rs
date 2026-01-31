use bitstream::{BitReader, BitVecWriter, BitWriter};

#[test]
fn bounded_writer_roundtrip_bits() {
    let mut buf = [0u8; 8];
    let mut writer = BitWriter::new(&mut buf);
    writer.write_bits(0b1010, 4).unwrap();
    writer.write_bits(0xAB, 8).unwrap();
    let bytes_used = writer.finish();

    let mut reader = BitReader::new(&buf[..bytes_used]);
    assert_eq!(reader.read_bits(4).unwrap(), 0b1010);
    assert_eq!(reader.read_bits(8).unwrap(), 0xAB);
}

#[test]
fn vec_writer_roundtrip_mixed() {
    let mut writer = BitVecWriter::new();
    writer.write_bit(true);
    writer.write_bits(0b1010, 4).unwrap();
    writer.align_to_byte();
    writer.write_u16_aligned(0xBEEF).unwrap();
    writer.write_varu32(300).unwrap();
    writer.write_vars32(-1).unwrap();
    let bytes = writer.finish();

    let mut reader = BitReader::new(&bytes);
    assert!(reader.read_bit().unwrap());
    assert_eq!(reader.read_bits(4).unwrap(), 0b1010);
    reader.align_to_byte().unwrap();
    assert_eq!(reader.read_u16_aligned().unwrap(), 0xBEEF);
    assert_eq!(reader.read_varu32().unwrap(), 300);
    assert_eq!(reader.read_vars32().unwrap(), -1);
}
