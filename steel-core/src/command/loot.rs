//! Command helpers for building loot predicate evaluation context.

use steel_registry::loot_table::{EntityRef, EntityRefFlags, WeatherState};

use crate::{entity::Entity, world::World};

pub(super) fn command_loot_entity_ref(entity: &dyn Entity) -> EntityRef<'_> {
    let living_entity = entity.as_living_entity();
    EntityRef {
        entity_type: Some(&entity.entity_type().key),
        flags: EntityRefFlags {
            is_on_fire: entity.is_on_fire(),
            is_sneaking: entity.is_crouching(),
            is_sprinting: living_entity.is_some_and(|entity| entity.is_sprinting()),
            is_swimming: entity.is_swimming(),
            is_baby: living_entity.is_some_and(|entity| entity.is_baby()),
        },
        equipment: None,
        custom_name: None,
    }
}

pub(super) fn command_loot_weather(world: &World) -> WeatherState {
    WeatherState {
        raining: world.is_raining(),
        thundering: world.is_thundering(),
    }
}
