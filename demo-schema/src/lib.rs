//! Demo schema and state definitions for the reference simulation.

use codec::{ComponentSnapshot, EntityId, EntitySnapshot, FieldValue, Snapshot, SnapshotTick};
use schema::{ChangePolicy, ComponentDef, ComponentId, FieldCodec, FieldDef, FieldId, Schema};

pub const POS_SCALE: u32 = 100;
pub const POS_MIN: i64 = -100_000;
pub const POS_MAX: i64 = 100_000;
pub const VEL_SCALE: u32 = 100;
pub const VEL_MIN: i64 = -10_000;
pub const VEL_MAX: i64 = 10_000;

const COMPONENT_ID: u16 = 1;
const FIELD_POS_X: u16 = 1;
const FIELD_POS_Y: u16 = 2;
const FIELD_POS_Z: u16 = 3;
const FIELD_VEL_X: u16 = 4;
const FIELD_VEL_Y: u16 = 5;
const FIELD_VEL_Z: u16 = 6;
const FIELD_YAW: u16 = 7;
const FIELD_FLAG_A: u16 = 8;
const FIELD_FLAG_B: u16 = 9;
const FIELD_FLAG_C: u16 = 10;

#[derive(Debug, Clone)]
pub struct DemoEntityState {
    pub id: EntityId,
    pub pos_q: [i64; 3],
    pub vel_q: [i64; 3],
    pub yaw: u16,
    pub flags: [bool; 3],
}

impl DemoEntityState {
    pub fn to_snapshot(&self) -> EntitySnapshot {
        EntitySnapshot {
            id: self.id,
            components: vec![ComponentSnapshot {
                id: component_id(),
                fields: vec![
                    FieldValue::FixedPoint(self.pos_q[0]),
                    FieldValue::FixedPoint(self.pos_q[1]),
                    FieldValue::FixedPoint(self.pos_q[2]),
                    FieldValue::FixedPoint(self.vel_q[0]),
                    FieldValue::FixedPoint(self.vel_q[1]),
                    FieldValue::FixedPoint(self.vel_q[2]),
                    FieldValue::UInt(self.yaw as u64),
                    FieldValue::Bool(self.flags[0]),
                    FieldValue::Bool(self.flags[1]),
                    FieldValue::Bool(self.flags[2]),
                ],
            }],
        }
    }
}

pub fn build_snapshot(tick: SnapshotTick, states: &[DemoEntityState]) -> Snapshot {
    let mut entities: Vec<EntitySnapshot> =
        states.iter().map(DemoEntityState::to_snapshot).collect();
    entities.sort_by_key(|entity| entity.id.raw());
    Snapshot { tick, entities }
}

pub fn demo_schema() -> Schema {
    let component = ComponentDef::new(component_id())
        .field(FieldDef::new(
            field_id(FIELD_POS_X),
            FieldCodec::fixed_point(POS_MIN, POS_MAX, POS_SCALE),
        ))
        .field(FieldDef::new(
            field_id(FIELD_POS_Y),
            FieldCodec::fixed_point(POS_MIN, POS_MAX, POS_SCALE),
        ))
        .field(FieldDef::new(
            field_id(FIELD_POS_Z),
            FieldCodec::fixed_point(POS_MIN, POS_MAX, POS_SCALE),
        ))
        .field(FieldDef::new(
            field_id(FIELD_VEL_X),
            FieldCodec::fixed_point(VEL_MIN, VEL_MAX, VEL_SCALE),
        ))
        .field(FieldDef::new(
            field_id(FIELD_VEL_Y),
            FieldCodec::fixed_point(VEL_MIN, VEL_MAX, VEL_SCALE),
        ))
        .field(FieldDef::new(
            field_id(FIELD_VEL_Z),
            FieldCodec::fixed_point(VEL_MIN, VEL_MAX, VEL_SCALE),
        ))
        .field(FieldDef::new(field_id(FIELD_YAW), FieldCodec::uint(12)))
        .field(FieldDef::new(field_id(FIELD_FLAG_A), FieldCodec::bool()))
        .field(FieldDef::new(field_id(FIELD_FLAG_B), FieldCodec::bool()))
        .field(
            FieldDef::new(field_id(FIELD_FLAG_C), FieldCodec::bool())
                .change(ChangePolicy::Threshold { threshold_q: 1 }),
        );
    Schema::new(vec![component]).expect("demo schema must be valid")
}

fn component_id() -> ComponentId {
    ComponentId::new(COMPONENT_ID).expect("component id must be non-zero")
}

fn field_id(value: u16) -> FieldId {
    FieldId::new(value).expect("field id must be non-zero")
}
