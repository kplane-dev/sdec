//! Field codec and change policy definitions.

use crate::FieldId;

/// Fixed-point quantization parameters (all integer-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedPoint {
    /// Minimum quantized value.
    pub min_q: i64,
    /// Maximum quantized value.
    pub max_q: i64,
    /// Units per 1.0 (e.g., 100 => 0.01 resolution).
    pub scale: u32,
}

impl FixedPoint {
    /// Creates a fixed-point configuration from quantized bounds and scale.
    #[must_use]
    pub const fn new(min_q: i64, max_q: i64, scale: u32) -> Self {
        Self {
            min_q,
            max_q,
            scale,
        }
    }
}

/// The encoding for a field (representation only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldCodec {
    /// Boolean (1 bit).
    Bool,

    /// Unsigned integer with fixed bit width.
    UInt { bits: u8 },

    /// Signed integer with fixed bit width.
    SInt { bits: u8 },

    /// Variable-length unsigned integer.
    VarUInt,

    /// Variable-length signed integer (zigzag encoded).
    VarSInt,

    /// Fixed-point number with quantization.
    FixedPoint(FixedPoint),
}

impl FieldCodec {
    /// Creates a boolean field codec.
    #[must_use]
    pub const fn bool() -> Self {
        Self::Bool
    }

    /// Creates an unsigned integer field codec.
    #[must_use]
    pub const fn uint(bits: u8) -> Self {
        Self::UInt { bits }
    }

    /// Creates a signed integer field codec.
    #[must_use]
    pub const fn sint(bits: u8) -> Self {
        Self::SInt { bits }
    }

    /// Creates a variable-length unsigned integer field codec.
    #[must_use]
    pub const fn var_uint() -> Self {
        Self::VarUInt
    }

    /// Creates a variable-length signed integer field codec.
    #[must_use]
    pub const fn var_sint() -> Self {
        Self::VarSInt
    }

    /// Creates a fixed-point field codec.
    #[must_use]
    pub const fn fixed_point(min_q: i64, max_q: i64, scale: u32) -> Self {
        Self::FixedPoint(FixedPoint::new(min_q, max_q, scale))
    }
}

/// Change detection policy for a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangePolicy {
    /// Always send when present in the component mask.
    Always,
    /// Send only if the quantized difference exceeds this threshold.
    Threshold { threshold_q: u32 },
}

/// Field definition within a component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldDef {
    pub id: FieldId,
    pub codec: FieldCodec,
    pub change: ChangePolicy,
}

impl FieldDef {
    /// Creates a field definition with the default change policy.
    #[must_use]
    pub const fn new(id: FieldId, codec: FieldCodec) -> Self {
        Self {
            id,
            codec,
            change: ChangePolicy::Always,
        }
    }

    /// Creates a field definition with a threshold policy.
    #[must_use]
    pub const fn with_threshold(id: FieldId, codec: FieldCodec, threshold_q: u32) -> Self {
        Self {
            id,
            codec,
            change: ChangePolicy::Threshold { threshold_q },
        }
    }

    /// Sets the change policy for a field definition.
    #[must_use]
    pub const fn change(mut self, change: ChangePolicy) -> Self {
        self.change = change;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FieldId;

    #[test]
    fn fixed_point_construction() {
        let fp = FixedPoint::new(-100, 200, 100);
        assert_eq!(fp.min_q, -100);
        assert_eq!(fp.max_q, 200);
        assert_eq!(fp.scale, 100);
    }

    #[test]
    fn field_codec_variants() {
        assert!(matches!(FieldCodec::bool(), FieldCodec::Bool));
        assert!(matches!(FieldCodec::uint(8), FieldCodec::UInt { bits: 8 }));
        assert!(matches!(FieldCodec::sint(8), FieldCodec::SInt { bits: 8 }));
        assert!(matches!(FieldCodec::var_uint(), FieldCodec::VarUInt));
        assert!(matches!(FieldCodec::var_sint(), FieldCodec::VarSInt));
        assert!(matches!(
            FieldCodec::fixed_point(-10, 10, 100),
            FieldCodec::FixedPoint(_)
        ));
    }

    #[test]
    fn field_def_default_change_policy() {
        let id = FieldId::new(1).unwrap();
        let field = FieldDef::new(id, FieldCodec::bool());
        assert_eq!(field.change, ChangePolicy::Always);
    }

    #[test]
    fn field_def_threshold_policy() {
        let id = FieldId::new(2).unwrap();
        let field = FieldDef::with_threshold(id, FieldCodec::uint(12), 5);
        assert_eq!(field.change, ChangePolicy::Threshold { threshold_q: 5 });
    }

    #[test]
    fn field_def_change_override() {
        let id = FieldId::new(3).unwrap();
        let field = FieldDef::new(id, FieldCodec::uint(8))
            .change(ChangePolicy::Threshold { threshold_q: 2 });
        assert_eq!(field.change, ChangePolicy::Threshold { threshold_q: 2 });
    }
}
