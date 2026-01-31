//! Core types for the codec.

/// A simulation tick number.
///
/// Ticks are monotonically increasing identifiers for simulation states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SnapshotTick(u32);

impl SnapshotTick {
    /// Creates a new snapshot tick.
    #[must_use]
    pub const fn new(tick: u32) -> Self {
        Self(tick)
    }

    /// Returns the raw tick value.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Returns `true` if this tick is zero (often used as "no baseline").
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }
}

impl From<u32> for SnapshotTick {
    fn from(tick: u32) -> Self {
        Self(tick)
    }
}

impl From<SnapshotTick> for u32 {
    fn from(tick: SnapshotTick) -> Self {
        tick.0
    }
}

/// A stable entity identifier.
///
/// Entity IDs are assigned by the simulation layer and must remain stable
/// for the lifetime of an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct EntityId(u32);

impl EntityId {
    /// Creates a new entity ID.
    #[must_use]
    pub const fn new(id: u32) -> Self {
        Self(id)
    }

    /// Returns the raw entity ID value.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl From<u32> for EntityId {
    fn from(id: u32) -> Self {
        Self(id)
    }
}

impl From<EntityId> for u32 {
    fn from(id: EntityId) -> Self {
        id.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // SnapshotTick tests
    #[test]
    fn snapshot_tick_new() {
        let tick = SnapshotTick::new(100);
        assert_eq!(tick.raw(), 100);
    }

    #[test]
    fn snapshot_tick_zero() {
        let zero = SnapshotTick::new(0);
        assert!(zero.is_zero());

        let nonzero = SnapshotTick::new(1);
        assert!(!nonzero.is_zero());
    }

    #[test]
    fn snapshot_tick_from_u32() {
        let tick: SnapshotTick = 42u32.into();
        assert_eq!(tick.raw(), 42);
    }

    #[test]
    fn snapshot_tick_into_u32() {
        let tick = SnapshotTick::new(99);
        let value: u32 = tick.into();
        assert_eq!(value, 99);
    }

    #[test]
    fn snapshot_tick_ordering() {
        let t1 = SnapshotTick::new(1);
        let t2 = SnapshotTick::new(2);
        let t3 = SnapshotTick::new(2);

        assert!(t1 < t2);
        assert!(t2 > t1);
        assert!(t2 == t3);
        assert!(t2 >= t3);
        assert!(t2 <= t3);
    }

    #[test]
    fn snapshot_tick_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(SnapshotTick::new(1));
        set.insert(SnapshotTick::new(2));
        set.insert(SnapshotTick::new(1)); // duplicate

        assert_eq!(set.len(), 2);
        assert!(set.contains(&SnapshotTick::new(1)));
        assert!(set.contains(&SnapshotTick::new(2)));
    }

    #[test]
    fn snapshot_tick_default() {
        let tick = SnapshotTick::default();
        assert_eq!(tick.raw(), 0);
        assert!(tick.is_zero());
    }

    #[test]
    fn snapshot_tick_equality() {
        let t1 = SnapshotTick::new(100);
        let t2 = SnapshotTick::new(100);
        let t3 = SnapshotTick::new(101);

        assert_eq!(t1, t2);
        assert_ne!(t1, t3);
    }

    #[test]
    fn snapshot_tick_clone_copy() {
        let tick = SnapshotTick::new(42);
        let copied = tick; // Copy
        assert_eq!(tick, copied);
    }

    #[test]
    fn snapshot_tick_debug() {
        let tick = SnapshotTick::new(123);
        let debug = format!("{tick:?}");
        assert!(debug.contains("123"));
    }

    #[test]
    fn snapshot_tick_const() {
        const TICK: SnapshotTick = SnapshotTick::new(42);
        assert_eq!(TICK.raw(), 42);
    }

    // EntityId tests
    #[test]
    fn entity_id_new() {
        let id = EntityId::new(42);
        assert_eq!(id.raw(), 42);
    }

    #[test]
    fn entity_id_from_u32() {
        let id: EntityId = 123u32.into();
        assert_eq!(id.raw(), 123);
    }

    #[test]
    fn entity_id_into_u32() {
        let id = EntityId::new(99);
        let value: u32 = id.into();
        assert_eq!(value, 99);
    }

    #[test]
    fn entity_id_ordering() {
        let id1 = EntityId::new(1);
        let id2 = EntityId::new(2);

        assert!(id1 < id2);
        assert!(id2 > id1);
    }

    #[test]
    fn entity_id_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(EntityId::new(1));
        set.insert(EntityId::new(2));

        assert!(set.contains(&EntityId::new(1)));
        assert!(!set.contains(&EntityId::new(3)));
    }

    #[test]
    fn entity_id_default() {
        let id = EntityId::default();
        assert_eq!(id.raw(), 0);
    }

    #[test]
    fn entity_id_equality() {
        let id1 = EntityId::new(42);
        let id2 = EntityId::new(42);
        let id3 = EntityId::new(43);

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);
    }

    #[test]
    fn entity_id_clone_copy() {
        let id = EntityId::new(42);
        let copied = id; // Copy
        assert_eq!(id, copied);
    }

    #[test]
    fn entity_id_const() {
        const ID: EntityId = EntityId::new(999);
        assert_eq!(ID.raw(), 999);
    }
}
