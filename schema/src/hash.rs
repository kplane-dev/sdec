//! Deterministic schema hashing.

use blake3::Hasher;

use crate::{ChangePolicy, FieldCodec, FixedPoint, Schema};

/// Computes a deterministic hash for schema validation.
#[must_use]
pub fn schema_hash(schema: &Schema) -> u64 {
    let mut hasher = Hasher::new();
    write_u32(&mut hasher, schema.components.len() as u32);

    for component in &schema.components {
        write_u16(&mut hasher, component.id.get());
        write_u32(&mut hasher, component.fields.len() as u32);

        for field in &component.fields {
            write_u16(&mut hasher, field.id.get());
            write_codec(&mut hasher, field.codec);
            write_change_policy(&mut hasher, field.change);
        }
    }

    let hash = hasher.finalize();
    let bytes = hash.as_bytes();
    u64::from_le_bytes(bytes[0..8].try_into().unwrap())
}

fn write_codec(hasher: &mut Hasher, codec: FieldCodec) {
    match codec {
        FieldCodec::Bool => {
            write_u8(hasher, 0);
        }
        FieldCodec::UInt { bits } => {
            write_u8(hasher, 1);
            write_u8(hasher, bits);
        }
        FieldCodec::SInt { bits } => {
            write_u8(hasher, 2);
            write_u8(hasher, bits);
        }
        FieldCodec::VarUInt => {
            write_u8(hasher, 3);
        }
        FieldCodec::VarSInt => {
            write_u8(hasher, 4);
        }
        FieldCodec::FixedPoint(fp) => {
            write_u8(hasher, 5);
            write_fixed_point(hasher, fp);
        }
    }
}

fn write_change_policy(hasher: &mut Hasher, policy: ChangePolicy) {
    match policy {
        ChangePolicy::Always => {
            write_u8(hasher, 0);
        }
        ChangePolicy::Threshold { threshold_q } => {
            write_u8(hasher, 1);
            write_u32(hasher, threshold_q);
        }
    }
}

fn write_fixed_point(hasher: &mut Hasher, fp: FixedPoint) {
    write_i64(hasher, fp.min_q);
    write_i64(hasher, fp.max_q);
    write_u32(hasher, fp.scale);
}

fn write_u8(hasher: &mut Hasher, value: u8) {
    hasher.update(&[value]);
}

fn write_u16(hasher: &mut Hasher, value: u16) {
    hasher.update(&value.to_le_bytes());
}

fn write_u32(hasher: &mut Hasher, value: u32) {
    hasher.update(&value.to_le_bytes());
}

fn write_i64(hasher: &mut Hasher, value: i64) {
    hasher.update(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ComponentDef, ComponentId, FieldCodec, FieldDef, FieldId, Schema};

    fn cid(value: u16) -> ComponentId {
        ComponentId::new(value).unwrap()
    }

    fn fid(value: u16) -> FieldId {
        FieldId::new(value).unwrap()
    }

    #[test]
    fn schema_hash_is_stable() {
        let component = ComponentDef::new(cid(1))
            .field(FieldDef::new(fid(1), FieldCodec::bool()))
            .field(FieldDef::with_threshold(fid(2), FieldCodec::uint(8), 2));
        let schema = Schema::new(vec![component]).unwrap();

        let hash1 = schema_hash(&schema);
        let hash2 = schema_hash(&schema);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn schema_hash_golden() {
        let component = ComponentDef::new(cid(10))
            .field(FieldDef::new(fid(1), FieldCodec::bool()))
            .field(FieldDef::new(fid(2), FieldCodec::sint(12)))
            .field(FieldDef::with_threshold(fid(3), FieldCodec::uint(5), 3))
            .field(FieldDef::new(
                fid(4),
                FieldCodec::fixed_point(-500, 500, 100),
            ));
        let schema = Schema::new(vec![component]).unwrap();

        let hash = schema_hash(&schema);
        assert_eq!(hash, 0x9320_BE45_8A81_5FCB);
    }

    #[test]
    fn schema_hash_changes_with_component_order() {
        let c1 = ComponentDef::new(cid(1)).field(FieldDef::new(fid(1), FieldCodec::bool()));
        let c2 = ComponentDef::new(cid(2)).field(FieldDef::new(fid(1), FieldCodec::uint(8)));

        let schema_a = Schema::new(vec![c1.clone(), c2.clone()]).unwrap();
        let schema_b = Schema::new(vec![c2, c1]).unwrap();

        assert_ne!(schema_hash(&schema_a), schema_hash(&schema_b));
    }

    #[test]
    fn schema_hash_changes_with_field_order() {
        let c1 = ComponentDef::new(cid(1))
            .field(FieldDef::new(fid(1), FieldCodec::bool()))
            .field(FieldDef::new(fid(2), FieldCodec::uint(8)));
        let c2 = ComponentDef::new(cid(1))
            .field(FieldDef::new(fid(2), FieldCodec::uint(8)))
            .field(FieldDef::new(fid(1), FieldCodec::bool()));

        let schema_a = Schema::new(vec![c1]).unwrap();
        let schema_b = Schema::new(vec![c2]).unwrap();

        assert_ne!(schema_hash(&schema_a), schema_hash(&schema_b));
    }
}
