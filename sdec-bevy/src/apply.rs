use anyhow::{anyhow, Result};
use bevy_ecs::prelude::World;
use codec::{DeltaUpdateEntity, EntityId, EntitySnapshot, FieldValue};

use crate::mapping::EntityMap;
use crate::schema::BevySchema;

pub fn apply_changes(
    schema: &BevySchema,
    world: &mut World,
    entities: &mut EntityMap,
    creates: &[EntitySnapshot],
    destroys: &[EntityId],
    updates: &[DeltaUpdateEntity],
) -> Result<()> {
    for destroy in destroys {
        if let Some(entity) = entities.entity(*destroy) {
            world.despawn(entity);
            entities.unregister(*destroy);
        }
    }

    for create in creates {
        let entity = world.spawn_empty().id();
        entities.register(create.id, entity);
        for component in &create.components {
            let adapter = schema
                .adapter_by_component(component.id)
                .ok_or_else(|| anyhow!("unknown component {:?}", component.id))?;
            adapter.insert_component(world, entity, &component.fields)?;
        }
    }

    apply_delta_updates(schema, world, entities, updates)?;
    Ok(())
}

pub fn apply_delta_updates(
    schema: &BevySchema,
    world: &mut World,
    entities: &mut EntityMap,
    updates: &[DeltaUpdateEntity],
) -> Result<()> {
    for update in updates {
        let Some(entity) = entities.entity(update.id) else {
            continue;
        };
        for component in &update.components {
            let adapter = schema
                .adapter_by_component(component.id)
                .ok_or_else(|| anyhow!("unknown component {:?}", component.id))?;
            let fields: Vec<(usize, FieldValue)> = component.fields.clone();
            adapter.apply_update(world, entity, &fields)?;
        }
    }
    Ok(())
}
