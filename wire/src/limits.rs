//! Configurable limits for bounded decoding.

/// Wire-level limits for packet decoding.
///
/// These limits are enforced during decoding to prevent resource exhaustion
/// attacks and ensure bounded memory usage. Section body parsing limits
/// belong to higher layers (codec/schema).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Maximum packet size in bytes.
    pub max_packet_bytes: usize,

    /// Maximum number of sections in a packet.
    pub max_sections: usize,

    /// Maximum length of a single section body in bytes.
    pub max_section_len: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            // 64 KB is generous for most realtime scenarios
            max_packet_bytes: 64 * 1024,

            // Typically only 3 sections (create, update, destroy)
            max_sections: 16,
            max_section_len: 32 * 1024,
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
            max_section_len: 1024,
        }
    }

    /// Creates limits with no restrictions (use with caution).
    #[must_use]
    pub const fn unlimited() -> Self {
        Self {
            max_packet_bytes: usize::MAX,
            max_sections: usize::MAX,
            max_section_len: usize::MAX,
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
    fn testing_limits_smaller() {
        let test_limits = Limits::for_testing();
        let default_limits = Limits::default();

        assert!(test_limits.max_packet_bytes < default_limits.max_packet_bytes);
        assert!(test_limits.max_sections < default_limits.max_sections);
        assert!(test_limits.max_section_len < default_limits.max_section_len);
    }

    #[test]
    fn testing_limits_values() {
        let limits = Limits::for_testing();
        assert_eq!(limits.max_packet_bytes, 4096);
        assert_eq!(limits.max_sections, 8);
        assert_eq!(limits.max_section_len, 1024);
    }

    #[test]
    fn unlimited_limits() {
        let limits = Limits::unlimited();
        assert_eq!(limits.max_packet_bytes, usize::MAX);
        assert_eq!(limits.max_sections, usize::MAX);
        assert_eq!(limits.max_section_len, usize::MAX);
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
