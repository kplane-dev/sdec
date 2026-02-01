use std::collections::HashMap;

use bevy_ecs::prelude::Entity;
use codec::EntityId;

#[derive(Debug, Default)]
pub struct EntityMap {
    next_id: u32,
    to_id: HashMap<Entity, EntityId>,
    to_entity: HashMap<EntityId, Entity>,
}

impl EntityMap {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn entity_id(&mut self, entity: Entity) -> EntityId {
        if let Some(id) = self.to_id.get(&entity) {
            return *id;
        }
        let next = self.next_id.saturating_add(1).max(1);
        self.next_id = next;
        let id = EntityId::new(next);
        self.to_id.insert(entity, id);
        self.to_entity.insert(id, entity);
        id
    }

    #[must_use]
    pub fn entity_id_known(&self, entity: Entity) -> Option<EntityId> {
        self.to_id.get(&entity).copied()
    }

    #[must_use]
    pub fn entity(&self, id: EntityId) -> Option<Entity> {
        self.to_entity.get(&id).copied()
    }

    pub fn register(&mut self, id: EntityId, entity: Entity) {
        self.to_id.insert(entity, id);
        self.to_entity.insert(id, entity);
        self.next_id = self.next_id.max(id.raw());
    }

    pub fn unregister(&mut self, id: EntityId) {
        if let Some(entity) = self.to_entity.remove(&id) {
            self.to_id.remove(&entity);
        }
    }
}
