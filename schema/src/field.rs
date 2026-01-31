//! Field codec definitions.

/// The kind of encoding for a field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FieldKind {
    /// Boolean (1 bit).
    Bool,

    /// Unsigned integer with fixed bit width.
    UInt {
        /// Number of bits (1-64).
        bits: u8,
    },

    /// Signed integer with fixed bit width.
    SInt {
        /// Number of bits (1-64).
        bits: u8,
    },

    /// Variable-length unsigned integer.
    VarUInt,

    /// Variable-length signed integer (zigzag encoded).
    VarSInt,

    /// Fixed-point number with quantization.
    FixedPoint {
        /// Minimum value.
        min: f32,
        /// Maximum value.
        max: f32,
        /// Number of bits for encoding.
        bits: u8,
    },
}

/// Codec configuration for a single field.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldCodec {
    /// The encoding kind.
    pub kind: FieldKind,

    /// Optional change threshold for delta encoding.
    /// If the difference is less than this threshold, the field is not sent.
    pub threshold: Option<f32>,
}

impl FieldCodec {
    /// Creates a boolean field codec.
    #[must_use]
    pub const fn bool() -> Self {
        Self {
            kind: FieldKind::Bool,
            threshold: None,
        }
    }

    /// Creates an unsigned integer field codec.
    #[must_use]
    pub const fn uint(bits: u8) -> Self {
        Self {
            kind: FieldKind::UInt { bits },
            threshold: None,
        }
    }

    /// Creates a signed integer field codec.
    #[must_use]
    pub const fn sint(bits: u8) -> Self {
        Self {
            kind: FieldKind::SInt { bits },
            threshold: None,
        }
    }

    /// Creates a variable-length unsigned integer field codec.
    #[must_use]
    pub const fn var_uint() -> Self {
        Self {
            kind: FieldKind::VarUInt,
            threshold: None,
        }
    }

    /// Creates a variable-length signed integer field codec.
    #[must_use]
    pub const fn var_sint() -> Self {
        Self {
            kind: FieldKind::VarSInt,
            threshold: None,
        }
    }

    /// Creates a fixed-point field codec.
    #[must_use]
    pub const fn fixed_point(min: f32, max: f32, bits: u8) -> Self {
        Self {
            kind: FieldKind::FixedPoint { min, max, bits },
            threshold: None,
        }
    }

    /// Sets the change threshold for delta encoding.
    #[must_use]
    pub const fn with_threshold(mut self, threshold: f32) -> Self {
        self.threshold = Some(threshold);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // FieldKind tests
    #[test]
    fn field_kind_bool() {
        let kind = FieldKind::Bool;
        assert!(matches!(kind, FieldKind::Bool));
    }

    #[test]
    fn field_kind_uint() {
        let kind = FieldKind::UInt { bits: 16 };
        assert!(matches!(kind, FieldKind::UInt { bits: 16 }));
    }

    #[test]
    fn field_kind_sint() {
        let kind = FieldKind::SInt { bits: 32 };
        assert!(matches!(kind, FieldKind::SInt { bits: 32 }));
    }

    #[test]
    fn field_kind_var_uint() {
        let kind = FieldKind::VarUInt;
        assert!(matches!(kind, FieldKind::VarUInt));
    }

    #[test]
    fn field_kind_var_sint() {
        let kind = FieldKind::VarSInt;
        assert!(matches!(kind, FieldKind::VarSInt));
    }

    #[test]
    fn field_kind_fixed_point() {
        let kind = FieldKind::FixedPoint {
            min: -100.0,
            max: 100.0,
            bits: 16,
        };
        match kind {
            FieldKind::FixedPoint { min, max, bits } => {
                assert!((min - (-100.0)).abs() < f32::EPSILON);
                assert!((max - 100.0).abs() < f32::EPSILON);
                assert_eq!(bits, 16);
            }
            _ => panic!("wrong kind"),
        }
    }

    #[test]
    fn field_kind_equality() {
        let k1 = FieldKind::UInt { bits: 8 };
        let k2 = FieldKind::UInt { bits: 8 };
        let k3 = FieldKind::UInt { bits: 16 };

        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
    }

    #[test]
    fn field_kind_clone_copy() {
        let kind = FieldKind::Bool;
        let copied = kind; // Copy
        assert_eq!(kind, copied);
    }

    // FieldCodec tests
    #[test]
    fn field_codec_bool() {
        let codec = FieldCodec::bool();
        assert!(matches!(codec.kind, FieldKind::Bool));
        assert!(codec.threshold.is_none());
    }

    #[test]
    fn field_codec_uint() {
        let codec = FieldCodec::uint(16);
        assert!(matches!(codec.kind, FieldKind::UInt { bits: 16 }));
        assert!(codec.threshold.is_none());
    }

    #[test]
    fn field_codec_sint() {
        let codec = FieldCodec::sint(32);
        assert!(matches!(codec.kind, FieldKind::SInt { bits: 32 }));
    }

    #[test]
    fn field_codec_var_uint() {
        let codec = FieldCodec::var_uint();
        assert!(matches!(codec.kind, FieldKind::VarUInt));
    }

    #[test]
    fn field_codec_var_sint() {
        let codec = FieldCodec::var_sint();
        assert!(matches!(codec.kind, FieldKind::VarSInt));
    }

    #[test]
    fn field_codec_fixed_point() {
        let codec = FieldCodec::fixed_point(-100.0, 100.0, 12);
        match codec.kind {
            FieldKind::FixedPoint { min, max, bits } => {
                assert!((min - (-100.0)).abs() < f32::EPSILON);
                assert!((max - 100.0).abs() < f32::EPSILON);
                assert_eq!(bits, 12);
            }
            _ => panic!("wrong kind"),
        }
    }

    #[test]
    fn field_codec_with_threshold() {
        let codec = FieldCodec::uint(8).with_threshold(0.5);
        assert_eq!(codec.threshold, Some(0.5));
    }

    #[test]
    fn field_codec_chained_threshold() {
        let codec = FieldCodec::fixed_point(-100.0, 100.0, 12).with_threshold(0.1);
        assert!(matches!(codec.kind, FieldKind::FixedPoint { bits: 12, .. }));
        assert_eq!(codec.threshold, Some(0.1));
    }

    #[test]
    fn field_codec_equality() {
        let c1 = FieldCodec::uint(8);
        let c2 = FieldCodec::uint(8);
        let c3 = FieldCodec::uint(16);

        assert_eq!(c1, c2);
        assert_ne!(c1, c3);
    }

    #[test]
    fn field_codec_threshold_equality() {
        let c1 = FieldCodec::uint(8).with_threshold(0.5);
        let c2 = FieldCodec::uint(8).with_threshold(0.5);
        let c3 = FieldCodec::uint(8).with_threshold(1.0);
        let c4 = FieldCodec::uint(8);

        assert_eq!(c1, c2);
        assert_ne!(c1, c3);
        assert_ne!(c1, c4);
    }

    #[test]
    fn field_codec_clone() {
        let codec = FieldCodec::uint(8).with_threshold(0.5);
        let cloned = codec.clone();
        assert_eq!(codec, cloned);
    }

    #[test]
    fn field_codec_const() {
        const CODEC: FieldCodec = FieldCodec::bool();
        assert!(matches!(CODEC.kind, FieldKind::Bool));
    }
}
