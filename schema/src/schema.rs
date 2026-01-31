//! Schema definitions and validation.

use std::collections::HashSet;

use crate::error::{SchemaError, SchemaResult};
use crate::{ChangePolicy, ComponentId, FieldCodec, FieldDef, FixedPoint};

/// A component definition within a schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentDef {
    pub id: ComponentId,
    pub fields: Vec<FieldDef>,
}

impl ComponentDef {
    /// Creates a new component with no fields.
    #[must_use]
    pub fn new(id: ComponentId) -> Self {
        Self {
            id,
            fields: Vec::new(),
        }
    }

    /// Creates a component with the provided fields.
    #[must_use]
    pub fn with_fields(id: ComponentId, fields: Vec<FieldDef>) -> Self {
        Self { id, fields }
    }

    /// Adds a field to the component.
    #[must_use]
    pub fn field(mut self, field: FieldDef) -> Self {
        self.fields.push(field);
        self
    }
}

/// A schema consisting of ordered components.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    pub components: Vec<ComponentDef>,
}

impl Schema {
    /// Creates a schema from components after validation.
    pub fn new(components: Vec<ComponentDef>) -> SchemaResult<Self> {
        let schema = Self { components };
        schema.validate()?;
        Ok(schema)
    }

    /// Creates a schema builder.
    #[must_use]
    pub fn builder() -> SchemaBuilder {
        SchemaBuilder {
            components: Vec::new(),
        }
    }

    /// Validates schema invariants.
    pub fn validate(&self) -> SchemaResult<()> {
        let mut component_ids = HashSet::new();
        for component in &self.components {
            if !component_ids.insert(component.id) {
                return Err(SchemaError::DuplicateComponentId { id: component.id });
            }

            let mut field_ids = HashSet::new();
            for field in &component.fields {
                if !field_ids.insert(field.id) {
                    return Err(SchemaError::DuplicateFieldId {
                        component: component.id,
                        field: field.id,
                    });
                }
                validate_field(field)?;
            }
        }
        Ok(())
    }
}

/// Builder for `Schema`.
#[derive(Debug, Default)]
pub struct SchemaBuilder {
    components: Vec<ComponentDef>,
}

impl SchemaBuilder {
    /// Adds a component definition.
    #[must_use]
    pub fn component(mut self, component: ComponentDef) -> Self {
        self.components.push(component);
        self
    }

    /// Builds the schema after validation.
    pub fn build(self) -> SchemaResult<Schema> {
        Schema::new(self.components)
    }
}

fn validate_field(field: &FieldDef) -> SchemaResult<()> {
    match field.codec {
        FieldCodec::UInt { bits } | FieldCodec::SInt { bits } => {
            if bits == 0 || bits > 64 {
                return Err(SchemaError::InvalidBitWidth { bits });
            }
        }
        FieldCodec::FixedPoint(fp) => {
            validate_fixed_point(fp)?;
        }
        FieldCodec::Bool | FieldCodec::VarUInt | FieldCodec::VarSInt => {}
    }

    if let ChangePolicy::Threshold { threshold_q } = field.change {
        if threshold_q == 0 {
            // Threshold zero is valid but redundant; allow it for now.
        }
    }
    Ok(())
}

fn validate_fixed_point(fp: FixedPoint) -> SchemaResult<()> {
    if fp.scale == 0 {
        return Err(SchemaError::InvalidFixedPointScale { scale: fp.scale });
    }
    if fp.min_q > fp.max_q {
        return Err(SchemaError::InvalidFixedPointRange {
            min_q: fp.min_q,
            max_q: fp.max_q,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FieldId;

    fn cid(value: u16) -> ComponentId {
        ComponentId::new(value).unwrap()
    }

    fn fid(value: u16) -> FieldId {
        FieldId::new(value).unwrap()
    }

    #[test]
    fn schema_builder_roundtrip() {
        let component = ComponentDef::new(cid(1))
            .field(FieldDef::new(fid(1), FieldCodec::bool()))
            .field(FieldDef::new(fid(2), FieldCodec::uint(8)));

        let schema = Schema::builder().component(component).build().unwrap();
        assert_eq!(schema.components.len(), 1);
    }

    #[test]
    fn schema_rejects_duplicate_component_ids() {
        let c1 = ComponentDef::new(cid(1));
        let c2 = ComponentDef::new(cid(1));
        let err = Schema::new(vec![c1, c2]).unwrap_err();
        assert!(matches!(err, SchemaError::DuplicateComponentId { .. }));
    }

    #[test]
    fn schema_rejects_duplicate_field_ids() {
        let component = ComponentDef::new(cid(1))
            .field(FieldDef::new(fid(1), FieldCodec::bool()))
            .field(FieldDef::new(fid(1), FieldCodec::uint(8)));
        let err = Schema::new(vec![component]).unwrap_err();
        assert!(matches!(err, SchemaError::DuplicateFieldId { .. }));
    }

    #[test]
    fn schema_rejects_invalid_bit_width() {
        let component = ComponentDef::new(cid(1)).field(FieldDef::new(fid(1), FieldCodec::uint(0)));
        let err = Schema::new(vec![component]).unwrap_err();
        assert!(matches!(err, SchemaError::InvalidBitWidth { .. }));
    }

    #[test]
    fn schema_rejects_invalid_fixed_point_scale() {
        let component = ComponentDef::new(cid(1))
            .field(FieldDef::new(fid(1), FieldCodec::fixed_point(-10, 10, 0)));
        let err = Schema::new(vec![component]).unwrap_err();
        assert!(matches!(err, SchemaError::InvalidFixedPointScale { .. }));
    }

    #[test]
    fn schema_rejects_invalid_fixed_point_range() {
        let component = ComponentDef::new(cid(1))
            .field(FieldDef::new(fid(1), FieldCodec::fixed_point(10, -10, 100)));
        let err = Schema::new(vec![component]).unwrap_err();
        assert!(matches!(err, SchemaError::InvalidFixedPointRange { .. }));
    }

    #[test]
    fn schema_allows_zero_threshold() {
        let component = ComponentDef::new(cid(1)).field(FieldDef::with_threshold(
            fid(1),
            FieldCodec::uint(8),
            0,
        ));
        let schema = Schema::new(vec![component]).unwrap();
        assert_eq!(schema.components.len(), 1);
    }
}
