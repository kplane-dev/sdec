use std::any::TypeId;
use std::marker::PhantomData;

use anyhow::{anyhow, Result};
use bevy_ecs::prelude::{Component, Entity, World};
use codec::{ComponentSnapshot, DeltaUpdateComponent, FieldValue};
use schema::{ChangePolicy, ComponentDef, ComponentId, FieldCodec, FieldDef, FieldId, Schema};

#[derive(Debug, Clone)]
pub struct ReplicatedField {
    pub id: u16,
    pub codec: FieldCodec,
    pub change: Option<ChangePolicy>,
}

pub trait ReplicatedComponent: Component {
    const COMPONENT_ID: u16;

    fn fields() -> Vec<ReplicatedField>;

    fn read_fields(&self) -> Vec<FieldValue>;

    fn apply_field(&mut self, index: usize, value: FieldValue) -> Result<()>;

    fn from_fields(fields: &[FieldValue]) -> Result<Self>
    where
        Self: Sized;
}

pub(crate) trait ComponentAdapter {
    fn type_id(&self) -> TypeId;
    fn component_id(&self) -> ComponentId;
    fn schema_def(&self) -> ComponentDef;
    fn snapshot_component(&self, world: &World, entity: Entity) -> Option<ComponentSnapshot>;
    fn update_component(&self, world: &World, entity: Entity) -> Option<DeltaUpdateComponent>;
    fn apply_update(
        &self,
        world: &mut World,
        entity: Entity,
        fields: &[(usize, FieldValue)],
    ) -> Result<()>;
    fn insert_component(
        &self,
        world: &mut World,
        entity: Entity,
        fields: &[FieldValue],
    ) -> Result<()>;
    fn added_entities(&self, world: &mut World) -> Vec<Entity>;
    fn changed_entities(&self, world: &mut World) -> Vec<Entity>;
    fn removed_entities(&self, world: &World) -> Vec<Entity>;
}

struct ComponentAdapterImpl<T: ReplicatedComponent> {
    component_id: ComponentId,
    fields: Vec<ReplicatedField>,
    _marker: PhantomData<T>,
}

impl<T: ReplicatedComponent> ComponentAdapterImpl<T> {
    fn new() -> Self {
        Self {
            component_id: ComponentId::new(T::COMPONENT_ID).expect("component id must be non-zero"),
            fields: T::fields(),
            _marker: PhantomData,
        }
    }

    fn snapshot_fields(&self, component: &T) -> Vec<FieldValue> {
        component.read_fields()
    }

    fn build_field_defs(&self) -> Vec<FieldDef> {
        self.fields
            .iter()
            .map(|field| {
                let mut def = FieldDef::new(
                    FieldId::new(field.id).expect("field id must be non-zero"),
                    field.codec,
                );
                if let Some(change) = field.change {
                    def = def.change(change);
                }
                def
            })
            .collect()
    }
}

impl<T: ReplicatedComponent> ComponentAdapter for ComponentAdapterImpl<T> {
    fn type_id(&self) -> TypeId {
        TypeId::of::<T>()
    }

    fn component_id(&self) -> ComponentId {
        self.component_id
    }

    fn schema_def(&self) -> ComponentDef {
        let mut def = ComponentDef::new(self.component_id);
        for field in self.build_field_defs() {
            def = def.field(field);
        }
        def
    }

    fn snapshot_component(&self, world: &World, entity: Entity) -> Option<ComponentSnapshot> {
        let component = world.get::<T>(entity)?;
        let fields = self.snapshot_fields(component);
        Some(ComponentSnapshot {
            id: self.component_id,
            fields,
        })
    }

    fn update_component(&self, world: &World, entity: Entity) -> Option<DeltaUpdateComponent> {
        let component = world.get::<T>(entity)?;
        let fields = self.snapshot_fields(component);
        let updates = fields.into_iter().enumerate().collect();
        Some(DeltaUpdateComponent {
            id: self.component_id,
            fields: updates,
        })
    }

    fn apply_update(
        &self,
        world: &mut World,
        entity: Entity,
        fields: &[(usize, FieldValue)],
    ) -> Result<()> {
        let mut component = world
            .get_mut::<T>(entity)
            .ok_or_else(|| anyhow!("missing component {:?}", self.component_id))?;
        for (index, value) in fields {
            component.apply_field(*index, *value)?;
        }
        Ok(())
    }

    fn insert_component(
        &self,
        world: &mut World,
        entity: Entity,
        fields: &[FieldValue],
    ) -> Result<()> {
        let component = T::from_fields(fields)?;
        world.entity_mut(entity).insert(component);
        Ok(())
    }

    fn added_entities(&self, world: &mut World) -> Vec<Entity> {
        let mut query = world.query_filtered::<Entity, bevy_ecs::query::Added<T>>();
        query.iter(world).collect()
    }

    fn changed_entities(&self, world: &mut World) -> Vec<Entity> {
        let mut query = world.query_filtered::<Entity, bevy_ecs::query::Changed<T>>();
        query.iter(world).collect()
    }

    fn removed_entities(&self, world: &World) -> Vec<Entity> {
        world.removed::<T>().collect()
    }
}

pub struct BevySchema {
    pub schema: Schema,
    adapters: Vec<Box<dyn ComponentAdapter>>,
}

impl BevySchema {
    #[must_use]
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    pub(crate) fn adapters(&self) -> &[Box<dyn ComponentAdapter>] {
        &self.adapters
    }

    pub(crate) fn adapter_by_component(
        &self,
        component_id: ComponentId,
    ) -> Option<&dyn ComponentAdapter> {
        self.adapters
            .iter()
            .map(|adapter| adapter.as_ref())
            .find(|adapter| adapter.component_id() == component_id)
    }

    pub(crate) fn snapshot_entity(&self, world: &World, entity: Entity) -> Vec<ComponentSnapshot> {
        self.adapters
            .iter()
            .filter_map(|adapter| adapter.snapshot_component(world, entity))
            .collect()
    }
}

#[derive(Default)]
pub struct BevySchemaBuilder {
    adapters: Vec<Box<dyn ComponentAdapter>>,
}

impl BevySchemaBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn component<T: ReplicatedComponent + 'static>(&mut self) -> &mut Self {
        let adapter = ComponentAdapterImpl::<T>::new();
        if self
            .adapters
            .iter()
            .any(|existing| existing.type_id() == adapter.type_id())
        {
            return self;
        }
        self.adapters.push(Box::new(adapter));
        self
    }

    pub fn build(self) -> Result<BevySchema> {
        let mut components = Vec::new();
        for adapter in &self.adapters {
            components.push(adapter.schema_def());
        }
        let schema = Schema::new(components).map_err(|err| anyhow!("{err:?}"))?;
        Ok(BevySchema {
            schema,
            adapters: self.adapters,
        })
    }
}
