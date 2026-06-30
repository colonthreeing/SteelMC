//! Command helpers for building loot predicate evaluation context.

use steel_registry::loot_table::{EntityRef, EntityRefFlags, WeatherState};
use steel_utils::random::{Random, legacy_random::LegacyRandom};

use crate::{entity::Entity, world::World};

pub(super) struct CommandLootRandom<'a>(&'a mut LegacyRandom);

impl<'a> CommandLootRandom<'a> {
    pub(super) const fn new(random: &'a mut LegacyRandom) -> Self {
        Self(random)
    }
}

impl rand::TryRng for CommandLootRandom<'_> {
    type Error = std::convert::Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(self.0.next_i32() as u32)
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok(self.0.next_i64() as u64)
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        let mut chunks = dst.chunks_exact_mut(8);
        for chunk in &mut chunks {
            chunk.copy_from_slice(&self.0.next_i64().to_le_bytes());
        }

        let remainder = chunks.into_remainder();
        if !remainder.is_empty() {
            let bytes = self.0.next_i64().to_le_bytes();
            remainder.copy_from_slice(&bytes[..remainder.len()]);
        }
        Ok(())
    }
}

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
