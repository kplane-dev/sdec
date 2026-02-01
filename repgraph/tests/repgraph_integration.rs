use codec::{
    apply_delta_snapshot, encode_delta_from_changes, CodecLimits, DeltaUpdateComponent,
    DeltaUpdateEntity, EntitySnapshot, FieldValue, SessionEncoder, Snapshot, SnapshotTick,
};
use repgraph::{ClientId, ClientView, ReplicationConfig, ReplicationGraph, Vec3, WorldView};
use schema::{ComponentDef, ComponentId, FieldCodec, FieldDef, FieldId, Schema};

#[derive(Clone)]
struct WorldState {
    entities: Vec<WorldEntity>,
}

#[derive(Clone)]
struct WorldEntity {
    id: codec::EntityId,
    position: Vec3,
    value: bool,
}

struct WorldAdapter<'a> {
    schema: &'a Schema,
    state: &'a WorldState,
}

impl<'a> WorldView for WorldAdapter<'a> {
    fn snapshot(&self, entity: codec::EntityId) -> EntitySnapshot {
        let world = self
            .state
            .entities
            .iter()
            .find(|entry| entry.id == entity)
            .expect("entity exists");
        EntitySnapshot {
            id: world.id,
            components: vec![codec::ComponentSnapshot {
                id: self.schema.components[0].id,
                fields: vec![FieldValue::Bool(world.value)],
            }],
        }
    }

    fn update(
        &self,
        entity: codec::EntityId,
        dirty_components: &[ComponentId],
    ) -> Option<DeltaUpdateEntity> {
        if dirty_components.is_empty() {
            return None;
        }
        let world = self
            .state
            .entities
            .iter()
            .find(|entry| entry.id == entity)?;
        Some(DeltaUpdateEntity {
            id: world.id,
            components: vec![DeltaUpdateComponent {
                id: self.schema.components[0].id,
                fields: vec![(0, FieldValue::Bool(world.value))],
            }],
        })
    }
}

fn schema_one_bool() -> Schema {
    let component = ComponentDef::new(ComponentId::new(1).unwrap())
        .field(FieldDef::new(FieldId::new(1).unwrap(), FieldCodec::bool()));
    Schema::new(vec![component]).unwrap()
}

#[test]
fn repgraph_delta_applies_for_client() {
    let schema = schema_one_bool();
    let limits = CodecLimits::for_testing();
    let mut session = SessionEncoder::new(&schema, &limits);

    let mut graph = ReplicationGraph::new(ReplicationConfig::default_limits());
    graph.upsert_client(
        ClientId(7),
        ClientView::new(
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            5.0,
        ),
    );

    let mut world = WorldState {
        entities: vec![
            WorldEntity {
                id: codec::EntityId::new(1),
                position: Vec3 {
                    x: 1.0,
                    y: 1.0,
                    z: 0.0,
                },
                value: false,
            },
            WorldEntity {
                id: codec::EntityId::new(2),
                position: Vec3 {
                    x: 50.0,
                    y: 0.0,
                    z: 0.0,
                },
                value: true,
            },
        ],
    };

    let adapter = WorldAdapter {
        schema: &schema,
        state: &world,
    };

    let baseline = Snapshot {
        tick: SnapshotTick::new(1),
        entities: Vec::new(),
    };

    graph.update_entity(
        world.entities[0].id,
        world.entities[0].position,
        &[schema.components[0].id],
    );
    graph.update_entity(
        world.entities[1].id,
        world.entities[1].position,
        &[schema.components[0].id],
    );

    let delta = graph.build_client_delta(ClientId(7), &adapter);
    graph.clear_dirty();

    let mut buf = [0u8; 256];
    let bytes = encode_delta_from_changes(
        &mut session,
        SnapshotTick::new(2),
        baseline.tick,
        &delta.creates,
        &delta.destroys,
        &delta.updates,
        &mut buf,
    )
    .unwrap();

    let applied = apply_delta_snapshot(
        &schema,
        &baseline,
        &buf[..bytes],
        &wire::Limits::for_testing(),
        &limits,
    )
    .unwrap();

    assert_eq!(applied.tick.raw(), 2);
    assert_eq!(applied.entities.len(), 1);
    assert_eq!(applied.entities[0].id.raw(), 1);

    world.entities[0].value = true;
    let adapter = WorldAdapter {
        schema: &schema,
        state: &world,
    };
    graph.update_entity(
        world.entities[0].id,
        world.entities[0].position,
        &[schema.components[0].id],
    );

    let delta = graph.build_client_delta(ClientId(7), &adapter);
    graph.clear_dirty();

    let bytes = encode_delta_from_changes(
        &mut session,
        SnapshotTick::new(3),
        applied.tick,
        &delta.creates,
        &delta.destroys,
        &delta.updates,
        &mut buf,
    )
    .unwrap();

    let applied = apply_delta_snapshot(
        &schema,
        &applied,
        &buf[..bytes],
        &wire::Limits::for_testing(),
        &limits,
    )
    .unwrap();

    let component = &applied.entities[0].components[0];
    assert_eq!(component.fields, vec![FieldValue::Bool(true)]);
}
