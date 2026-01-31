use bitstream::{BitReader, BitVecWriter};
use proptest::prelude::*;

#[derive(Clone, Debug)]
enum Op {
    Bit(bool),
    Bits { bits: u8, value: u64 },
    Align,
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    VarU32(u32),
    VarS32(i32),
}

fn mask_value(bits: u8, value: u64) -> u64 {
    if bits >= 64 {
        value
    } else {
        let mask = (1u64 << bits) - 1;
        value & mask
    }
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        any::<bool>().prop_map(Op::Bit),
        (1u8..=64, any::<u64>()).prop_map(|(bits, value)| Op::Bits {
            bits,
            value: mask_value(bits, value),
        }),
        Just(Op::Align),
        any::<u8>().prop_map(Op::U8),
        any::<u16>().prop_map(Op::U16),
        any::<u32>().prop_map(Op::U32),
        any::<u64>().prop_map(Op::U64),
        any::<u32>().prop_map(Op::VarU32),
        any::<i32>().prop_map(Op::VarS32),
    ]
}

proptest! {
    #[test]
    fn prop_roundtrip_ops(ops in prop::collection::vec(op_strategy(), 1..64)) {
        let mut writer = BitVecWriter::new();

        for op in &ops {
            match op {
                Op::Bit(b) => {
                    writer.write_bit(*b);
                }
                Op::Bits { bits, value } => {
                    writer.write_bits(*value, *bits).unwrap();
                }
                Op::Align => {
                    writer.align_to_byte();
                }
                Op::U8(v) => {
                    if writer.bits_written() % 8 != 0 {
                        writer.align_to_byte();
                    }
                    writer.write_u8_aligned(*v).unwrap();
                }
                Op::U16(v) => {
                    if writer.bits_written() % 8 != 0 {
                        writer.align_to_byte();
                    }
                    writer.write_u16_aligned(*v).unwrap();
                }
                Op::U32(v) => {
                    if writer.bits_written() % 8 != 0 {
                        writer.align_to_byte();
                    }
                    writer.write_u32_aligned(*v).unwrap();
                }
                Op::U64(v) => {
                    if writer.bits_written() % 8 != 0 {
                        writer.align_to_byte();
                    }
                    writer.write_u64_aligned(*v).unwrap();
                }
                Op::VarU32(v) => {
                    if writer.bits_written() % 8 != 0 {
                        writer.align_to_byte();
                    }
                    writer.write_varu32(*v).unwrap();
                }
                Op::VarS32(v) => {
                    if writer.bits_written() % 8 != 0 {
                        writer.align_to_byte();
                    }
                    writer.write_vars32(*v).unwrap();
                }
            }
        }

        let bytes = writer.finish();
        let mut reader = BitReader::new(&bytes);

        for op in &ops {
            match op {
                Op::Bit(b) => {
                    prop_assert_eq!(reader.read_bit().unwrap(), *b);
                }
                Op::Bits { bits, value } => {
                    prop_assert_eq!(reader.read_bits(*bits).unwrap(), *value);
                }
                Op::Align => {
                    reader.align_to_byte().unwrap();
                }
                Op::U8(v) => {
                    if reader.bit_position() % 8 != 0 {
                        reader.align_to_byte().unwrap();
                    }
                    prop_assert_eq!(reader.read_u8_aligned().unwrap(), *v);
                }
                Op::U16(v) => {
                    if reader.bit_position() % 8 != 0 {
                        reader.align_to_byte().unwrap();
                    }
                    prop_assert_eq!(reader.read_u16_aligned().unwrap(), *v);
                }
                Op::U32(v) => {
                    if reader.bit_position() % 8 != 0 {
                        reader.align_to_byte().unwrap();
                    }
                    prop_assert_eq!(reader.read_u32_aligned().unwrap(), *v);
                }
                Op::U64(v) => {
                    if reader.bit_position() % 8 != 0 {
                        reader.align_to_byte().unwrap();
                    }
                    prop_assert_eq!(reader.read_u64_aligned().unwrap(), *v);
                }
                Op::VarU32(v) => {
                    if reader.bit_position() % 8 != 0 {
                        reader.align_to_byte().unwrap();
                    }
                    prop_assert_eq!(reader.read_varu32().unwrap(), *v);
                }
                Op::VarS32(v) => {
                    if reader.bit_position() % 8 != 0 {
                        reader.align_to_byte().unwrap();
                    }
                    prop_assert_eq!(reader.read_vars32().unwrap(), *v);
                }
            }
        }
    }
}
