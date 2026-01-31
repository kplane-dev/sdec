//! Limits for codec-level decoding.

/// Codec-specific limits enforced during snapshot decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecLimits {
    /// Maximum number of entities in an ENTITY_CREATE section.
    pub max_entities_create: usize,
    /// Maximum number of entities in an ENTITY_UPDATE section.
    pub max_entities_update: usize,
    /// Maximum number of entities in an ENTITY_DESTROY section.
    pub max_entities_destroy: usize,
    /// Maximum number of components per entity.
    pub max_components_per_entity: usize,
    /// Maximum number of fields per component.
    pub max_fields_per_component: usize,
    /// Maximum number of bytes in a section body.
    pub max_section_bytes: usize,
    /// Maximum number of entities after applying a delta.
    pub max_total_entities_after_apply: usize,
}

impl Default for CodecLimits {
    fn default() -> Self {
        Self {
            max_entities_create: 1024,
            max_entities_update: 2048,
            max_entities_destroy: 1024,
            max_components_per_entity: 64,
            max_fields_per_component: 64,
            max_section_bytes: 64 * 1024,
            max_total_entities_after_apply: 4096,
        }
    }
}

impl CodecLimits {
    /// Creates limits suitable for testing with smaller values.
    #[must_use]
    pub const fn for_testing() -> Self {
        Self {
            max_entities_create: 32,
            max_entities_update: 64,
            max_entities_destroy: 32,
            max_components_per_entity: 16,
            max_fields_per_component: 16,
            max_section_bytes: 4096,
            max_total_entities_after_apply: 128,
        }
    }

    /// Creates limits with no restrictions (use with caution).
    #[must_use]
    pub const fn unlimited() -> Self {
        Self {
            max_entities_create: usize::MAX,
            max_entities_update: usize::MAX,
            max_entities_destroy: usize::MAX,
            max_components_per_entity: usize::MAX,
            max_fields_per_component: usize::MAX,
            max_section_bytes: usize::MAX,
            max_total_entities_after_apply: usize::MAX,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_limits_are_reasonable() {
        let limits = CodecLimits::default();
        assert!(limits.max_entities_create >= 128);
        assert!(limits.max_section_bytes >= 1024);
    }

    #[test]
    fn testing_limits_smaller() {
        let test_limits = CodecLimits::for_testing();
        let default_limits = CodecLimits::default();
        assert!(test_limits.max_entities_create < default_limits.max_entities_create);
        assert!(test_limits.max_section_bytes < default_limits.max_section_bytes);
    }

    #[test]
    fn unlimited_limits() {
        let limits = CodecLimits::unlimited();
        assert_eq!(limits.max_entities_create, usize::MAX);
        assert_eq!(limits.max_section_bytes, usize::MAX);
    }
}
