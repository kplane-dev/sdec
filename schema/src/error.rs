//! Schema validation errors.

/// Result type for schema operations.
pub type SchemaResult<T> = Result<T, SchemaError>;

/// Errors that can occur when building or validating a schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaError {
    /// Duplicate component ID in a schema.
    DuplicateComponentId { id: crate::ComponentId },

    /// Duplicate field ID within a component.
    DuplicateFieldId {
        component: crate::ComponentId,
        field: crate::FieldId,
    },

    /// Invalid bit width for fixed-width integers.
    InvalidBitWidth { bits: u8 },

    /// Fixed-point scale must be non-zero.
    InvalidFixedPointScale { scale: u32 },

    /// Fixed-point min/max range is invalid.
    InvalidFixedPointRange { min_q: i64, max_q: i64 },
}
