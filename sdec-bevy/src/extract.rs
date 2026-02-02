use std::collections::{HashMap, HashSet};

use bevy_ecs::prelude::{Entity, World};
use codec::{DeltaUpdateEntity, EntityId, EntitySnapshot};
use schema::ComponentId;

use crate::mapping::EntityMap;
use crate::schema::BevySchema;

#[derive(Debug, Default)]
pub struct BevyChangeSet {
    pub creates: Vec<EntitySnapshot>,
    pub destroys: Vec<EntityId>,
    pub updates: Vec<DeltaUpdateEntity>,
}

#[derive(Debug, Default)]
pub(crate) struct ExtractScratch {
    create_entities: HashSet<Entity>,
    update_entities: HashMap<Entity, Vec<ComponentId>>,
    destroys: HashSet<EntityId>,
}

pub fn extract_changes(
    schema: &BevySchema,
    world: &mut World,
    entities: &mut EntityMap,
) -> BevyChangeSet {
    let mut scratch = ExtractScratch::default();
    let mut changes = BevyChangeSet::default();
    extract_changes_with_scratch(schema, world, entities, &mut scratch, &mut changes);
    changes
}

pub(crate) fn extract_changes_with_scratch(
    schema: &BevySchema,
    world: &mut World,
    entities: &mut EntityMap,
    scratch: &mut ExtractScratch,
    out: &mut BevyChangeSet,
) {
    scratch.create_entities.clear();
    scratch.update_entities.clear();
    scratch.destroys.clear();
    out.creates.clear();
    out.updates.clear();
    out.destroys.clear();

    for adapter in schema.adapters() {
        for entity in adapter.added_entities(world) {
            scratch.create_entities.insert(entity);
        }
        for entity in adapter.changed_entities(world) {
            scratch
                .update_entities
                .entry(entity)
                .or_default()
                .push(adapter.component_id());
        }
        for entity in adapter.removed_entities(world) {
            if let Some(id) = entities.entity_id_known(entity) {
                scratch.destroys.insert(id);
            }
        }
    }

    out.creates.reserve(scratch.create_entities.len());
    for entity in scratch.create_entities.iter().copied() {
        let id = entities.entity_id(entity);
        let components = schema.snapshot_entity(world, entity);
        if components.is_empty() {
            continue;
        }
        out.creates.push(EntitySnapshot { id, components });
    }

    out.updates.reserve(scratch.update_entities.len());
    for (entity, components) in scratch.update_entities.iter() {
        if scratch.create_entities.contains(entity) {
            continue;
        }
        let id = entities.entity_id(*entity);
        let mut delta_components = Vec::new();
        for component_id in components {
            if let Some(adapter) = schema.adapter_by_component(*component_id) {
                if let Some(component_update) = adapter.update_component(world, *entity) {
                    delta_components.push(component_update);
                }
            }
        }
        if !delta_components.is_empty() {
            out.updates.push(DeltaUpdateEntity {
                id,
                components: delta_components,
            });
        }
    }

    out.destroys.reserve(scratch.destroys.len());
    out.destroys.extend(scratch.destroys.iter().copied());
    out.destroys.sort_by_key(|id| id.raw());
    out.creates.sort_by_key(|entity| entity.id.raw());
    out.updates.sort_by_key(|entity| entity.id.raw());
}
