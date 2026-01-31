//! Error types for codec operations.

use std::fmt;

/// Result type for codec operations.
pub type CodecResult<T> = Result<T, CodecError>;

/// Errors that can occur during snapshot/delta encoding/decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    /// Wire format error.
    Wire(wire::WireError),

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

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(e) => write!(f, "wire error: {e}"),
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

impl std::error::Error for CodecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Wire(e) => Some(e),
            _ => None,
        }
    }
}

impl From<wire::WireError> for CodecError {
    fn from(err: wire::WireError) -> Self {
        Self::Wire(err)
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
        let wire_err = wire::WireError::InvalidMagic { found: 0x1234 };
        let codec_err: CodecError = wire_err.into();
        assert!(matches!(codec_err, CodecError::Wire(_)));
    }

    #[test]
    fn error_source_wire() {
        let wire_err = wire::WireError::InvalidMagic { found: 0x1234 };
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
