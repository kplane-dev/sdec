//! Replication schema and field codec definitions for the sdec codec.
//!
//! This crate defines how game state is represented for replication:
//! - Schema model for entity types, components, and fields
//! - Field codecs (bool, integers, fixed-point, varints)
//! - Quantization and threshold configuration
//! - Deterministic schema hashing
//!
//! # Design Principles
//!
//! - **Runtime-first** - v0 uses runtime schema building, derive macros come later.
//! - **Explicit schemas** - No reflection on arbitrary Rust types.
//! - **Deterministic hashing** - Schema hash is stable given the same definition.

mod field;
mod hash;

pub use field::{FieldCodec, FieldKind};
pub use hash::schema_hash;

/// A component ID within a schema.
pub type ComponentId = u16;

/// A field ID within a component.
pub type FieldId = u16;

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn public_api_exports() {
        // Verify all expected items are exported
        let _ = FieldCodec::bool();
        let _ = FieldKind::Bool;
        let _ = schema_hash(&[]);

        // Type aliases
        let _: ComponentId = 0;
        let _: FieldId = 0;
    }

    #[test]
    fn field_codec_basic_usage() {
        let codec = FieldCodec::bool();
        assert!(matches!(codec.kind, FieldKind::Bool));
    }

    #[test]
    fn schema_hash_stub() {
        assert_eq!(schema_hash(&[1, 2, 3]), 0);
    }

    #[test]
    fn component_id_and_field_id_sizes() {
        // Verify the type sizes match WIRE_FORMAT.md
        assert_eq!(size_of::<ComponentId>(), 2);
        assert_eq!(size_of::<FieldId>(), 2);
    }
}
