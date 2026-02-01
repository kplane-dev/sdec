use std::collections::BTreeMap;

use codec::{
    apply_delta_snapshot, CodecLimits, ComponentSnapshot, DeltaUpdateComponent, DeltaUpdateEntity,
    EntityId, EntitySnapshot, FieldValue, SessionEncoder, Snapshot, SnapshotTick,
};
use repgraph::{ClientId, ClientView, ReplicationConfig, ReplicationGraph, Vec3, WorldView};
use schema::{ComponentDef, ComponentId, FieldCodec, FieldDef, FieldId, Schema};

#[derive(Clone)]
struct EntityState {
    pos: Vec3,
    flag: bool,
    value: u64,
}

struct TestWorld {
    current: BTreeMap<EntityId, EntityState>,
    previous: BTreeMap<EntityId, EntityState>,
    component_id: ComponentId,
}

impl TestWorld {
    fn new(component_id: ComponentId) -> Self {
        Self {
            current: BTreeMap::new(),
            previous: BTreeMap::new(),
            component_id,
        }
    }

    fn set(&mut self, id: EntityId, state: EntityState) {
        self.current.insert(id, state);
    }

    fn commit_previous(&mut self) {
        self.previous = self.current.clone();
    }

    fn snapshot_for_view(&self, tick: SnapshotTick, view: ClientView) -> Snapshot {
        let mut entities = Vec::new();
        let radius_sq = view.radius * view.radius;
        for (id, state) in &self.current {
            if state.pos.distance_sq(view.position) <= radius_sq {
                entities.push(self.build_snapshot(*id, state));
            }
        }
        Snapshot { tick, entities }
    }

    fn build_snapshot(&self, id: EntityId, state: &EntityState) -> EntitySnapshot {
        EntitySnapshot {
            id,
            components: vec![ComponentSnapshot {
                id: self.component_id,
                fields: vec![FieldValue::Bool(state.flag), FieldValue::UInt(state.value)],
            }],
        }
    }
}

impl WorldView for TestWorld {
    fn snapshot(&self, entity: EntityId) -> EntitySnapshot {
        let state = self.current.get(&entity).expect("missing state");
        self.build_snapshot(entity, state)
    }

    fn update(
        &self,
        entity: EntityId,
        dirty_components: &[ComponentId],
    ) -> Option<DeltaUpdateEntity> {
        if !dirty_components.contains(&self.component_id) {
            return None;
        }
        let prev = self.previous.get(&entity)?;
        let curr = self.current.get(&entity)?;
        let mut fields = Vec::new();
        if prev.flag != curr.flag {
            fields.push((0, FieldValue::Bool(curr.flag)));
        }
        if prev.value != curr.value {
            fields.push((1, FieldValue::UInt(curr.value)));
        }
        if fields.is_empty() {
            return None;
        }
        Some(DeltaUpdateEntity {
            id: entity,
            components: vec![DeltaUpdateComponent {
                id: self.component_id,
                fields,
            }],
        })
    }
}

fn test_schema() -> Schema {
    let component = ComponentDef::new(ComponentId::new(1).unwrap())
        .field(FieldDef::new(FieldId::new(1).unwrap(), FieldCodec::bool()))
        .field(FieldDef::new(
            FieldId::new(2).unwrap(),
            FieldCodec::uint(16),
        ));
    Schema::new(vec![component]).unwrap()
}

#[test]
fn repgraph_drives_delta_from_changes() {
    let schema = test_schema();
    let component_id = schema.components[0].id;
    let view = ClientView::new(
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        10.0,
    );

    let mut graph = ReplicationGraph::new(ReplicationConfig::default_limits());
    graph.upsert_client(ClientId(1), view);

    let mut world = TestWorld::new(component_id);
    world.set(
        EntityId::new(1),
        EntityState {
            pos: Vec3 {
                x: 1.0,
                y: 1.0,
                z: 0.0,
            },
            flag: false,
            value: 10,
        },
    );
    world.set(
        EntityId::new(2),
        EntityState {
            pos: Vec3 {
                x: 100.0,
                y: 0.0,
                z: 0.0,
            },
            flag: false,
            value: 20,
        },
    );
    world.set(
        EntityId::new(3),
        EntityState {
            pos: Vec3 {
                x: 2.0,
                y: 0.0,
                z: 0.0,
            },
            flag: true,
            value: 30,
        },
    );

    for (id, state) in &world.current {
        graph.update_entity(*id, state.pos, &[]);
    }

    let limits = CodecLimits::for_testing();
    let mut session = SessionEncoder::new(&schema, &limits);
    let baseline = Snapshot {
        tick: SnapshotTick::new(1),
        entities: Vec::new(),
    };
    let tick1 = SnapshotTick::new(2);
    let delta1 = graph.build_client_delta(ClientId(1), &world);
    let mut buf = [0u8; 2048];
    let len1 = codec::encode_delta_from_changes(
        &mut session,
        tick1,
        baseline.tick,
        &delta1.creates,
        &delta1.destroys,
        &delta1.updates,
        &mut buf,
    )
    .unwrap();
    let applied1 = apply_delta_snapshot(
        &schema,
        &baseline,
        &buf[..len1],
        &wire::Limits::for_testing(),
        &CodecLimits::for_testing(),
    )
    .unwrap();
    let expected1 = world.snapshot_for_view(tick1, view);
    assert_eq!(applied1, expected1);

    world.commit_previous();

    world.set(
        EntityId::new(1),
        EntityState {
            pos: Vec3 {
                x: 1.0,
                y: 1.0,
                z: 0.0,
            },
            flag: true,
            value: 11,
        },
    );
    world.set(
        EntityId::new(2),
        EntityState {
            pos: Vec3 {
                x: 3.0,
                y: 0.0,
                z: 0.0,
            },
            flag: false,
            value: 20,
        },
    );
    world.set(
        EntityId::new(3),
        EntityState {
            pos: Vec3 {
                x: 50.0,
                y: 0.0,
                z: 0.0,
            },
            flag: true,
            value: 30,
        },
    );

    graph.update_entity(
        EntityId::new(1),
        world.current[&EntityId::new(1)].pos,
        &[component_id],
    );
    graph.update_entity(EntityId::new(2), world.current[&EntityId::new(2)].pos, &[]);
    graph.update_entity(EntityId::new(3), world.current[&EntityId::new(3)].pos, &[]);

    let tick2 = SnapshotTick::new(3);
    let delta2 = graph.build_client_delta(ClientId(1), &world);
    let len2 = codec::encode_delta_from_changes(
        &mut session,
        tick2,
        applied1.tick,
        &delta2.creates,
        &delta2.destroys,
        &delta2.updates,
        &mut buf,
    )
    .unwrap();
    let applied2 = apply_delta_snapshot(
        &schema,
        &applied1,
        &buf[..len2],
        &wire::Limits::for_testing(),
        &CodecLimits::for_testing(),
    )
    .unwrap();
    let expected2 = world.snapshot_for_view(tick2, view);
    assert_eq!(applied2, expected2);
}
