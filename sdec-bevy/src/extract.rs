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

pub fn extract_changes(
    schema: &BevySchema,
    world: &mut World,
    entities: &mut EntityMap,
) -> BevyChangeSet {
    let mut create_entities = HashSet::new();
    let mut update_entities: HashMap<Entity, Vec<ComponentId>> = HashMap::new();
    let mut destroys: HashSet<EntityId> = HashSet::new();

    for adapter in schema.adapters() {
        for entity in adapter.added_entities(world) {
            create_entities.insert(entity);
        }
        for entity in adapter.changed_entities(world) {
            update_entities
                .entry(entity)
                .or_default()
                .push(adapter.component_id());
        }
        for entity in adapter.removed_entities(world) {
            if let Some(id) = entities.entity_id_known(entity) {
                destroys.insert(id);
            }
        }
    }

    let mut creates = Vec::new();
    for entity in create_entities.iter().copied() {
        let id = entities.entity_id(entity);
        let components = schema.snapshot_entity(world, entity);
        if components.is_empty() {
            continue;
        }
        creates.push(EntitySnapshot { id, components });
    }

    let mut updates = Vec::new();
    for (entity, components) in update_entities {
        if create_entities.contains(&entity) {
            continue;
        }
        let id = entities.entity_id(entity);
        let mut delta_components = Vec::new();
        for adapter in schema.adapters() {
            if !components.contains(&adapter.component_id()) {
                continue;
            }
            if let Some(component_update) = adapter.update_component(world, entity) {
                delta_components.push(component_update);
            }
        }
        if !delta_components.is_empty() {
            updates.push(DeltaUpdateEntity {
                id,
                components: delta_components,
            });
        }
    }

    let mut destroys_vec: Vec<EntityId> = destroys.into_iter().collect();
    destroys_vec.sort_by_key(|id| id.raw());

    creates.sort_by_key(|entity| entity.id.raw());
    updates.sort_by_key(|entity| entity.id.raw());

    BevyChangeSet {
        creates,
        destroys: destroys_vec,
        updates,
    }
}
