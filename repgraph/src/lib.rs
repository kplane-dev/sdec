//! Replication graph: decide what to encode, not how.
//!
//! This crate provides interest management and per-client change list
//! generation that feeds directly into `codec::encode_delta_from_changes`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use codec::{DeltaUpdateEntity, EntitySnapshot};
use schema::ComponentId;

/// Client identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientId(pub u32);

/// Basic 3D vector for spatial queries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    #[must_use]
    pub fn distance_sq(self, other: Self) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;
        dx * dx + dy * dy + dz * dz
    }
}

/// Budget caps for per-client deltas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientBudget {
    pub max_creates: usize,
    pub max_updates: usize,
    pub max_destroys: usize,
}

impl ClientBudget {
    #[must_use]
    pub fn unlimited() -> Self {
        Self {
            max_creates: usize::MAX,
            max_updates: usize::MAX,
            max_destroys: usize::MAX,
        }
    }
}

/// Client view configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClientView {
    pub position: Vec3,
    pub radius: f32,
    pub budget: ClientBudget,
}

impl ClientView {
    #[must_use]
    pub fn new(position: Vec3, radius: f32) -> Self {
        Self {
            position,
            radius,
            budget: ClientBudget::unlimited(),
        }
    }
}

/// Replication graph configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplicationConfig {
    /// Maximum entities tracked globally (hard safety cap).
    pub max_entities: usize,
}

impl ReplicationConfig {
    #[must_use]
    pub fn default_limits() -> Self {
        Self {
            max_entities: 1_000_000,
        }
    }
}

/// Per-client delta output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientDelta {
    pub creates: Vec<EntitySnapshot>,
    pub destroys: Vec<codec::EntityId>,
    pub updates: Vec<DeltaUpdateEntity>,
}

impl ClientDelta {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.creates.is_empty() && self.destroys.is_empty() && self.updates.is_empty()
    }
}

/// World view adapter used to build snapshot/update payloads.
pub trait WorldView {
    /// Build a full entity snapshot for creates.
    fn snapshot(&self, entity: codec::EntityId) -> EntitySnapshot;

    /// Build a delta update from dirty components. Return `None` to skip.
    fn update(
        &self,
        entity: codec::EntityId,
        dirty_components: &[ComponentId],
    ) -> Option<DeltaUpdateEntity>;
}

#[derive(Debug, Clone)]
struct EntityEntry {
    position: Vec3,
    priority: u8,
    dirty_components: Vec<ComponentId>,
}

#[derive(Debug, Clone)]
struct ClientState {
    view: ClientView,
    known_entities: BTreeSet<codec::EntityId>,
}

/// Replication graph with basic spatial relevance and dirty tracking.
#[derive(Debug, Clone)]
pub struct ReplicationGraph {
    config: ReplicationConfig,
    entities: BTreeMap<codec::EntityId, EntityEntry>,
    removed_entities: BTreeSet<codec::EntityId>,
    clients: HashMap<ClientId, ClientState>,
}

impl ReplicationGraph {
    #[must_use]
    pub fn new(config: ReplicationConfig) -> Self {
        Self {
            config,
            entities: BTreeMap::new(),
            removed_entities: BTreeSet::new(),
            clients: HashMap::new(),
        }
    }

    /// Add or update a tracked entity.
    pub fn update_entity(
        &mut self,
        entity: codec::EntityId,
        position: Vec3,
        dirty_components: &[ComponentId],
    ) {
        if self.entities.len() >= self.config.max_entities && !self.entities.contains_key(&entity) {
            return;
        }
        let entry = self.entities.entry(entity).or_insert(EntityEntry {
            position,
            priority: 0,
            dirty_components: Vec::new(),
        });
        entry.position = position;
        push_unique_components(&mut entry.dirty_components, dirty_components);
    }

    /// Set entity priority (higher is more important).
    pub fn set_entity_priority(&mut self, entity: codec::EntityId, priority: u8) {
        if let Some(entry) = self.entities.get_mut(&entity) {
            entry.priority = priority;
        }
    }

    /// Remove an entity and schedule destroy for all clients.
    pub fn remove_entity(&mut self, entity: codec::EntityId) {
        self.entities.remove(&entity);
        self.removed_entities.insert(entity);
    }

    /// Update or insert client view configuration.
    pub fn upsert_client(&mut self, client: ClientId, view: ClientView) {
        self.clients
            .entry(client)
            .and_modify(|state| state.view = view)
            .or_insert(ClientState {
                view,
                known_entities: BTreeSet::new(),
            });
    }

    /// Remove a client and its known-entity state.
    pub fn remove_client(&mut self, client: ClientId) {
        self.clients.remove(&client);
    }

    /// Build the per-client delta (creates/destroys/updates) from current graph state.
    pub fn build_client_delta(&mut self, client: ClientId, world: &impl WorldView) -> ClientDelta {
        let Some(state) = self.clients.get_mut(&client) else {
            return ClientDelta {
                creates: Vec::new(),
                destroys: Vec::new(),
                updates: Vec::new(),
            };
        };

        let radius_sq = state.view.radius * state.view.radius;
        let mut relevant: BTreeSet<codec::EntityId> = BTreeSet::new();
        for (id, entry) in &self.entities {
            if entry.position.distance_sq(state.view.position) <= radius_sq {
                relevant.insert(*id);
            }
        }

        let mut creates = Vec::new();
        let mut updates = Vec::new();
        for id in relevant.iter().copied() {
            if !state.known_entities.contains(&id) {
                creates.push(world.snapshot(id));
                continue;
            }
            if let Some(entry) = self.entities.get(&id) {
                if !entry.dirty_components.is_empty() {
                    if let Some(update) = world.update(id, &entry.dirty_components) {
                        updates.push(update);
                    }
                }
            }
        }

        let mut destroys: Vec<codec::EntityId> = state
            .known_entities
            .difference(&relevant)
            .copied()
            .collect();
        for removed in &self.removed_entities {
            if state.known_entities.contains(removed) && !destroys.contains(removed) {
                destroys.push(*removed);
            }
        }
        destroys.sort_by_key(|id| id.raw());

        apply_budget(&mut creates, &mut updates, &mut destroys, state.view.budget);

        let mut next_known = state.known_entities.clone();
        for destroy in &destroys {
            next_known.remove(destroy);
        }
        for create in &creates {
            next_known.insert(create.id);
        }
        state.known_entities = next_known;

        ClientDelta {
            creates,
            destroys,
            updates,
        }
    }

    /// Clear dirty flags after all clients have been processed for a tick.
    pub fn clear_dirty(&mut self) {
        for entry in self.entities.values_mut() {
            entry.dirty_components.clear();
        }
    }

    /// Clear pending removals after all clients have processed destroys.
    pub fn clear_removed(&mut self) {
        self.removed_entities.clear();
    }
}

fn push_unique_components(target: &mut Vec<ComponentId>, new_components: &[ComponentId]) {
    for component in new_components {
        if !target.contains(component) {
            target.push(*component);
        }
    }
}

fn apply_budget(
    creates: &mut Vec<EntitySnapshot>,
    updates: &mut Vec<DeltaUpdateEntity>,
    destroys: &mut Vec<codec::EntityId>,
    budget: ClientBudget,
) {
    if creates.len() > budget.max_creates {
        creates.truncate(budget.max_creates);
    }
    if updates.len() > budget.max_updates {
        updates.truncate(budget.max_updates);
    }
    if destroys.len() > budget.max_destroys {
        destroys.truncate(budget.max_destroys);
    }
}
