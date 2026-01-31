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
//! - **Runtime-first** - the initial release uses runtime schema building; derive macros come later.
//! - **Explicit schemas** - No reflection on arbitrary Rust types.
//! - **Deterministic hashing** - Schema hash is stable given the same definition.

mod error;
mod field;
mod hash;
mod schema;

use std::num::NonZeroU16;

pub use error::{SchemaError, SchemaResult};
pub use field::{ChangePolicy, FieldCodec, FieldDef, FixedPoint};
pub use hash::schema_hash;
pub use schema::{ComponentDef, Schema, SchemaBuilder};

/// A component ID within a schema (non-zero).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ComponentId(NonZeroU16);

impl ComponentId {
    /// Creates a new component ID. Returns `None` if `value` is zero.
    #[must_use]
    pub const fn new(value: u16) -> Option<Self> {
        match NonZeroU16::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Returns the underlying numeric value.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0.get()
    }
}

/// A field ID within a component (non-zero).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FieldId(NonZeroU16);

impl FieldId {
    /// Creates a new field ID. Returns `None` if `value` is zero.
    #[must_use]
    pub const fn new(value: u16) -> Option<Self> {
        match NonZeroU16::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Returns the underlying numeric value.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn public_api_exports() {
        // Verify all expected items are exported
        let _ = FieldCodec::bool();
        let _ = ChangePolicy::Always;
        let _ = FieldDef::new(FieldId::new(1).unwrap(), FieldCodec::bool());
        let _ = Schema::builder();
        let _ = schema_hash(&Schema::new(Vec::new()).unwrap());

        // Type aliases
        let _: ComponentId = ComponentId::new(1).unwrap();
        let _: FieldId = FieldId::new(1).unwrap();
    }

    #[test]
    fn field_codec_basic_usage() {
        let codec = FieldCodec::bool();
        assert!(matches!(codec, FieldCodec::Bool));
    }

    #[test]
    fn schema_hash_basic() {
        let schema = Schema::new(Vec::new()).unwrap();
        assert_ne!(schema_hash(&schema), 0);
    }

    #[test]
    fn component_id_and_field_id_sizes() {
        // Verify the type sizes match WIRE_FORMAT.md
        assert_eq!(size_of::<ComponentId>(), 2);
        assert_eq!(size_of::<FieldId>(), 2);
    }

    #[test]
    fn component_id_zero_is_invalid() {
        assert!(ComponentId::new(0).is_none());
    }

    #[test]
    fn field_id_zero_is_invalid() {
        assert!(FieldId::new(0).is_none());
    }
}
