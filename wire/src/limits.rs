//! Configurable limits for bounded decoding.

/// Limits for packet decoding.
///
/// These limits are enforced during decoding to prevent resource exhaustion
/// attacks and ensure bounded memory usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Maximum packet size in bytes.
    pub max_packet_bytes: usize,

    /// Maximum number of sections in a packet.
    pub max_sections: usize,

    /// Maximum number of entities in an `ENTITY_CREATE` section.
    pub max_entities_create: usize,

    /// Maximum number of entities in an `ENTITY_UPDATE` section.
    pub max_entities_update: usize,

    /// Maximum number of entities in an `ENTITY_DESTROY` section.
    pub max_entities_destroy: usize,

    /// Maximum number of components per entity.
    pub max_components_per_entity: usize,

    /// Maximum number of fields per component.
    pub max_fields_per_component: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            // 64 KB is generous for most FPS scenarios
            max_packet_bytes: 64 * 1024,

            // Typically only 3 sections (create, update, destroy)
            max_sections: 16,

            // Reasonable defaults for FPS games
            max_entities_create: 256,
            max_entities_update: 1024,
            max_entities_destroy: 256,

            // Typical ECS limits
            max_components_per_entity: 64,
            max_fields_per_component: 64,
        }
    }
}

impl Limits {
    /// Creates limits suitable for testing with smaller values.
    #[must_use]
    pub const fn for_testing() -> Self {
        Self {
            max_packet_bytes: 4096,
            max_sections: 8,
            max_entities_create: 32,
            max_entities_update: 64,
            max_entities_destroy: 32,
            max_components_per_entity: 16,
            max_fields_per_component: 16,
        }
    }

    /// Creates limits with no restrictions (use with caution).
    #[must_use]
    pub const fn unlimited() -> Self {
        Self {
            max_packet_bytes: usize::MAX,
            max_sections: usize::MAX,
            max_entities_create: usize::MAX,
            max_entities_update: usize::MAX,
            max_entities_destroy: usize::MAX,
            max_components_per_entity: usize::MAX,
            max_fields_per_component: usize::MAX,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_limits_packet_bytes() {
        let limits = Limits::default();
        assert_eq!(limits.max_packet_bytes, 64 * 1024);
    }

    #[test]
    fn default_limits_sections() {
        let limits = Limits::default();
        assert_eq!(limits.max_sections, 16);
    }

    #[test]
    fn default_limits_entities() {
        let limits = Limits::default();
        assert_eq!(limits.max_entities_create, 256);
        assert_eq!(limits.max_entities_update, 1024);
        assert_eq!(limits.max_entities_destroy, 256);
        // Update should be >= create (updates are more common)
        assert!(limits.max_entities_update >= limits.max_entities_create);
    }

    #[test]
    fn default_limits_components_fields() {
        let limits = Limits::default();
        assert_eq!(limits.max_components_per_entity, 64);
        assert_eq!(limits.max_fields_per_component, 64);
    }

    #[test]
    fn testing_limits_smaller() {
        let test_limits = Limits::for_testing();
        let default_limits = Limits::default();

        assert!(test_limits.max_packet_bytes < default_limits.max_packet_bytes);
        assert!(test_limits.max_sections < default_limits.max_sections);
        assert!(test_limits.max_entities_create < default_limits.max_entities_create);
        assert!(test_limits.max_entities_update < default_limits.max_entities_update);
        assert!(test_limits.max_entities_destroy < default_limits.max_entities_destroy);
    }

    #[test]
    fn testing_limits_values() {
        let limits = Limits::for_testing();
        assert_eq!(limits.max_packet_bytes, 4096);
        assert_eq!(limits.max_sections, 8);
        assert_eq!(limits.max_entities_create, 32);
        assert_eq!(limits.max_entities_update, 64);
        assert_eq!(limits.max_entities_destroy, 32);
        assert_eq!(limits.max_components_per_entity, 16);
        assert_eq!(limits.max_fields_per_component, 16);
    }

    #[test]
    fn unlimited_limits() {
        let limits = Limits::unlimited();
        assert_eq!(limits.max_packet_bytes, usize::MAX);
        assert_eq!(limits.max_sections, usize::MAX);
        assert_eq!(limits.max_entities_create, usize::MAX);
        assert_eq!(limits.max_entities_update, usize::MAX);
        assert_eq!(limits.max_entities_destroy, usize::MAX);
        assert_eq!(limits.max_components_per_entity, usize::MAX);
        assert_eq!(limits.max_fields_per_component, usize::MAX);
    }

    #[test]
    fn limits_equality() {
        let l1 = Limits::default();
        let l2 = Limits::default();
        let l3 = Limits::for_testing();

        assert_eq!(l1, l2);
        assert_ne!(l1, l3);
    }

    #[test]
    fn limits_clone() {
        let limits = Limits::default();
        let cloned = limits.clone();
        assert_eq!(limits, cloned);
    }

    #[test]
    fn limits_debug() {
        let limits = Limits::default();
        let debug = format!("{limits:?}");
        assert!(debug.contains("Limits"));
        assert!(debug.contains("max_packet_bytes"));
    }

    #[test]
    fn limits_const_constructible() {
        const LIMITS: Limits = Limits::for_testing();
        assert_eq!(LIMITS.max_packet_bytes, 4096);
    }
}
