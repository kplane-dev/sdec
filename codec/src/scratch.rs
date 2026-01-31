//! Reusable scratch buffers for codec operations.

/// Scratch buffers for delta encoding.
#[derive(Debug, Default)]
pub struct CodecScratch {
    component_changed: Vec<bool>,
    field_mask: Vec<bool>,
}

impl CodecScratch {
    /// Creates a new scratch buffer with no pre-allocated capacity.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn ensure_component_capacity(&mut self, components: usize) {
        if self.component_changed.len() < components {
            self.component_changed.resize(components, false);
        }
    }

    fn ensure_field_capacity(&mut self, fields: usize) {
        if self.field_mask.len() < fields {
            self.field_mask.resize(fields, false);
        }
    }
}

impl CodecScratch {
    pub(crate) fn component_and_field_masks_mut(
        &mut self,
        components: usize,
        fields: usize,
    ) -> (&mut [bool], &mut [bool]) {
        self.ensure_component_capacity(components);
        self.ensure_field_capacity(fields);
        let component_changed = &mut self.component_changed[..components];
        let field_mask = &mut self.field_mask[..fields];
        (component_changed, field_mask)
    }
}
