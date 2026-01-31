//! Error types for codec operations.

use std::fmt;

use schema::{ComponentId, FieldId};

/// Result type for codec operations.
pub type CodecResult<T> = Result<T, CodecError>;

/// Errors that can occur during snapshot/delta encoding/decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    /// Wire format error.
    Wire(wire::DecodeError),

    /// Bitstream error.
    Bitstream(bitstream::BitError),

    /// Output buffer is too small.
    OutputTooSmall { needed: usize, available: usize },

    /// Schema hash mismatch.
    SchemaMismatch { expected: u64, found: u64 },

    /// Limits exceeded.
    LimitsExceeded {
        kind: LimitKind,
        limit: usize,
        actual: usize,
    },

    /// Invalid mask data.
    InvalidMask { kind: MaskKind, reason: MaskReason },

    /// Invalid field value for the schema.
    InvalidValue {
        component: ComponentId,
        field: FieldId,
        reason: ValueReason,
    },

    /// Entities are not provided in deterministic order.
    InvalidEntityOrder { previous: u32, current: u32 },

    /// Section body had trailing bits after parsing.
    TrailingSectionData {
        section: wire::SectionTag,
        remaining_bits: usize,
    },

    /// Unexpected section for the current packet type.
    UnexpectedSection { section: wire::SectionTag },

    /// Duplicate section encountered.
    DuplicateSection { section: wire::SectionTag },

    /// Multiple update encodings present in one packet.
    DuplicateUpdateEncoding,

    /// Baseline tick does not match the packet.
    BaselineTickMismatch { expected: u32, found: u32 },

    /// Baseline tick not found in history.
    BaselineNotFound {
        /// The requested baseline tick.
        requested_tick: u32,
    },

    /// Entity not found when applying delta.
    EntityNotFound {
        /// The missing entity ID.
        entity_id: u32,
    },

    /// Component not found when applying delta.
    ComponentNotFound {
        /// The entity ID.
        entity_id: u32,
        /// The missing component ID.
        component_id: u16,
    },

    /// Duplicate entity in create section.
    DuplicateEntity {
        /// The duplicate entity ID.
        entity_id: u32,
    },

    /// Entity already exists when creating.
    EntityAlreadyExists {
        /// The existing entity ID.
        entity_id: u32,
    },
}

/// Specific limit that was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitKind {
    EntitiesCreate,
    EntitiesUpdate,
    EntitiesDestroy,
    TotalEntitiesAfterApply,
    ComponentsPerEntity,
    FieldsPerComponent,
    SectionBytes,
}

/// Mask validation error kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskKind {
    ComponentMask,
    FieldMask { component: ComponentId },
}

/// Details for invalid mask errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskReason {
    NotEnoughBits { expected: usize, available: usize },
    FieldCountMismatch { expected: usize, actual: usize },
    MissingField { field: FieldId },
    UnknownComponent { component: ComponentId },
    InvalidComponentId { raw: u16 },
    InvalidFieldIndex { field_index: usize, max: usize },
    ComponentPresenceMismatch { component: ComponentId },
    EmptyFieldMask { component: ComponentId },
}

/// Details for invalid value errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueReason {
    UnsignedOutOfRange {
        bits: u8,
        value: u64,
    },
    SignedOutOfRange {
        bits: u8,
        value: i64,
    },
    VarUIntOutOfRange {
        value: u64,
    },
    VarSIntOutOfRange {
        value: i64,
    },
    FixedPointOutOfRange {
        min_q: i64,
        max_q: i64,
        value: i64,
    },
    TypeMismatch {
        expected: &'static str,
        found: &'static str,
    },
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(e) => write!(f, "wire error: {e}"),
            Self::Bitstream(e) => write!(f, "bitstream error: {e}"),
            Self::OutputTooSmall { needed, available } => {
                write!(f, "output too small: need {needed}, have {available}")
            }
            Self::SchemaMismatch { expected, found } => {
                write!(
                    f,
                    "schema hash mismatch: expected 0x{expected:016X}, found 0x{found:016X}"
                )
            }
            Self::LimitsExceeded {
                kind,
                limit,
                actual,
            } => {
                write!(f, "{kind} limit exceeded: {actual} > {limit}")
            }
            Self::InvalidMask { kind, reason } => {
                write!(f, "invalid {kind}: {reason}")
            }
            Self::InvalidValue {
                component,
                field,
                reason,
            } => {
                write!(f, "invalid value for {component:?}:{field:?}: {reason}")
            }
            Self::InvalidEntityOrder { previous, current } => {
                write!(f, "entity order invalid: {previous} then {current}")
            }
            Self::TrailingSectionData {
                section,
                remaining_bits,
            } => {
                write!(
                    f,
                    "trailing data in section {section:?}: {remaining_bits} bits"
                )
            }
            Self::UnexpectedSection { section } => {
                write!(f, "unexpected section {section:?} in full snapshot")
            }
            Self::DuplicateSection { section } => {
                write!(f, "duplicate section {section:?} in packet")
            }
            Self::DuplicateUpdateEncoding => {
                write!(f, "multiple update encodings present in packet")
            }
            Self::BaselineTickMismatch { expected, found } => {
                write!(
                    f,
                    "baseline tick mismatch: expected {expected}, found {found}"
                )
            }
            Self::BaselineNotFound { requested_tick } => {
                write!(f, "baseline tick {requested_tick} not found in history")
            }
            Self::EntityNotFound { entity_id } => {
                write!(f, "entity {entity_id} not found")
            }
            Self::ComponentNotFound {
                entity_id,
                component_id,
            } => {
                write!(
                    f,
                    "component {component_id} not found on entity {entity_id}"
                )
            }
            Self::DuplicateEntity { entity_id } => {
                write!(f, "duplicate entity {entity_id} in create section")
            }
            Self::EntityAlreadyExists { entity_id } => {
                write!(f, "entity {entity_id} already exists")
            }
        }
    }
}

impl fmt::Display for LimitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::EntitiesCreate => "entities",
            Self::EntitiesUpdate => "update entities",
            Self::EntitiesDestroy => "destroy entities",
            Self::TotalEntitiesAfterApply => "total entities",
            Self::ComponentsPerEntity => "components per entity",
            Self::FieldsPerComponent => "fields per component",
            Self::SectionBytes => "section bytes",
        };
        write!(f, "{name}")
    }
}

impl fmt::Display for MaskKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ComponentMask => write!(f, "component mask"),
            Self::FieldMask { component } => write!(f, "field mask for {component:?}"),
        }
    }
}

impl fmt::Display for MaskReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotEnoughBits {
                expected,
                available,
            } => {
                write!(f, "need {expected} bits, have {available}")
            }
            Self::FieldCountMismatch { expected, actual } => {
                write!(f, "expected {expected} fields, got {actual}")
            }
            Self::MissingField { field } => {
                write!(f, "missing field {field:?} in full snapshot")
            }
            Self::UnknownComponent { component } => {
                write!(f, "unknown component {component:?} in snapshot")
            }
            Self::InvalidComponentId { raw } => {
                write!(f, "invalid component id {raw} in snapshot")
            }
            Self::InvalidFieldIndex { field_index, max } => {
                write!(f, "field index {field_index} exceeds max {max}")
            }
            Self::ComponentPresenceMismatch { component } => {
                write!(f, "component presence mismatch for {component:?}")
            }
            Self::EmptyFieldMask { component } => {
                write!(f, "empty field mask for {component:?} is invalid")
            }
        }
    }
}

impl fmt::Display for ValueReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsignedOutOfRange { bits, value } => {
                write!(f, "unsigned value {value} does not fit in {bits} bits")
            }
            Self::SignedOutOfRange { bits, value } => {
                write!(f, "signed value {value} does not fit in {bits} bits")
            }
            Self::VarUIntOutOfRange { value } => {
                write!(f, "varuint value {value} exceeds u32::MAX")
            }
            Self::VarSIntOutOfRange { value } => {
                write!(f, "varsint value {value} exceeds i32 range")
            }
            Self::FixedPointOutOfRange {
                min_q,
                max_q,
                value,
            } => {
                write!(f, "fixed-point value {value} outside [{min_q}, {max_q}]")
            }
            Self::TypeMismatch { expected, found } => {
                write!(f, "expected {expected} but got {found}")
            }
        }
    }
}

impl std::error::Error for CodecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Wire(e) => Some(e),
            Self::Bitstream(e) => Some(e),
            _ => None,
        }
    }
}

impl From<wire::DecodeError> for CodecError {
    fn from(err: wire::DecodeError) -> Self {
        Self::Wire(err)
    }
}

impl From<bitstream::BitError> for CodecError {
    fn from(err: bitstream::BitError) -> Self {
        Self::Bitstream(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_baseline_not_found() {
        let err = CodecError::BaselineNotFound { requested_tick: 42 };
        let msg = err.to_string();
        assert!(msg.contains("42"), "should mention tick");
        assert!(msg.contains("baseline"), "should mention baseline");
    }

    #[test]
    fn error_display_entity_not_found() {
        let err = CodecError::EntityNotFound { entity_id: 123 };
        let msg = err.to_string();
        assert!(msg.contains("123"), "should mention entity id");
    }

    #[test]
    fn error_display_component_not_found() {
        let err = CodecError::ComponentNotFound {
            entity_id: 10,
            component_id: 5,
        };
        let msg = err.to_string();
        assert!(msg.contains("10"), "should mention entity id");
        assert!(msg.contains('5'), "should mention component id");
    }

    #[test]
    fn error_display_duplicate_entity() {
        let err = CodecError::DuplicateEntity { entity_id: 42 };
        let msg = err.to_string();
        assert!(msg.contains("42"), "should mention entity id");
        assert!(msg.contains("duplicate"), "should mention duplicate");
    }

    #[test]
    fn error_display_entity_already_exists() {
        let err = CodecError::EntityAlreadyExists { entity_id: 99 };
        let msg = err.to_string();
        assert!(msg.contains("99"), "should mention entity id");
        assert!(msg.contains("exists"), "should mention exists");
    }

    #[test]
    fn error_from_wire_error() {
        let wire_err = wire::DecodeError::InvalidMagic { found: 0x1234 };
        let codec_err: CodecError = wire_err.into();
        assert!(matches!(codec_err, CodecError::Wire(_)));
    }

    #[test]
    fn error_from_bitstream_error() {
        let bit_err = bitstream::BitError::UnexpectedEof {
            requested: 1,
            available: 0,
        };
        let codec_err: CodecError = bit_err.into();
        assert!(matches!(codec_err, CodecError::Bitstream(_)));
    }

    #[test]
    fn error_source_wire() {
        let wire_err = wire::DecodeError::InvalidMagic { found: 0x1234 };
        let codec_err = CodecError::Wire(wire_err);
        let source = std::error::Error::source(&codec_err);
        assert!(source.is_some(), "should have a source");
    }

    #[test]
    fn error_source_none_for_others() {
        let err = CodecError::EntityNotFound { entity_id: 1 };
        let source = std::error::Error::source(&err);
        assert!(source.is_none(), "non-wrapped errors should have no source");
    }

    #[test]
    fn error_equality() {
        let err1 = CodecError::EntityNotFound { entity_id: 42 };
        let err2 = CodecError::EntityNotFound { entity_id: 42 };
        let err3 = CodecError::EntityNotFound { entity_id: 43 };

        assert_eq!(err1, err2);
        assert_ne!(err1, err3);
    }

    #[test]
    fn error_is_std_error() {
        fn assert_error<E: std::error::Error>() {}
        assert_error::<CodecError>();
    }
}
