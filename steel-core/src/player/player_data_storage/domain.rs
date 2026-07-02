use std::io::Cursor;

use simdnbt::{ToNbtTag, borrow::read_compound as read_borrowed_compound, owned::NbtTag};
use tokio::io;
use wincode::{SchemaRead, SchemaWrite};

use super::codec::{decode_file, encode_file};
use crate::chunk_saver::PersistentEntity;
use crate::player::player_data::{
    PLAYER_DATA_VERSION, PersistentAbilities, PersistentPlayerData, PersistentRootVehicle,
    PersistentSlot,
};
use steel_registry::item_stack::ItemStack;

const PLAYER_MAGIC: [u8; 4] = *b"STLP";
pub(super) const PLAYER_STORAGE_VERSION: u16 = 7;

#[derive(SchemaWrite, SchemaRead)]
pub(super) struct PlayerDataFile {
    pub(super) data_version: i32,
    pub(super) pos: [f64; 3],
    pub(super) motion: [f64; 3],
    pub(super) rotation: [f32; 2],
    pub(super) on_ground: bool,
    pub(super) fall_flying: bool,
    pub(super) remaining_fire_ticks: i32,
    pub(super) ticks_frozen: i32,
    pub(super) is_in_powder_snow: bool,
    pub(super) was_in_powder_snow: bool,
    pub(super) has_visual_fire: bool,
    pub(super) health: f32,
    pub(super) game_mode: i32,
    pub(super) prev_game_mode: Option<i32>,
    pub(super) abilities: AbilitiesFile,
    pub(super) inventory: Vec<SlotFile>,
    pub(super) ender_chest: Vec<SlotFile>,
    pub(super) selected_slot: i32,
    pub(super) world: String,
    pub(super) food_level: i32,
    pub(super) food_saturation_level: f32,
    pub(super) food_exhaustion_level: f32,
    pub(super) food_tick_timer: i32,
    pub(super) experience_level: i32,
    pub(super) experience_progress: f32,
    pub(super) experience_total: i32,
    pub(super) score: i32,
    pub(super) root_vehicle: Option<RootVehicleFile>,
}

#[derive(SchemaWrite, SchemaRead)]
pub(super) struct RootVehicleFile {
    pub(super) attach: [u8; 16],
    pub(super) entity: PersistentEntity,
}

#[derive(SchemaWrite, SchemaRead)]
pub(super) struct AbilitiesFile {
    pub(super) invulnerable: bool,
    pub(super) flying: bool,
    pub(super) may_fly: bool,
    pub(super) instabuild: bool,
    pub(super) may_build: bool,
    pub(super) flying_speed: f32,
    pub(super) walking_speed: f32,
}

#[derive(SchemaWrite, SchemaRead)]
pub(super) struct SlotFile {
    pub(super) slot: i8,
    pub(super) item_nbt: Vec<u8>,
}

impl PlayerDataFile {
    pub(super) fn from_persistent(data: &PersistentPlayerData) -> io::Result<Self> {
        Ok(Self {
            data_version: data.data_version,
            pos: data.pos,
            motion: data.motion,
            rotation: data.rotation,
            on_ground: data.on_ground,
            fall_flying: data.fall_flying,
            remaining_fire_ticks: data.remaining_fire_ticks,
            ticks_frozen: data.ticks_frozen,
            is_in_powder_snow: data.is_in_powder_snow,
            was_in_powder_snow: data.was_in_powder_snow,
            has_visual_fire: data.has_visual_fire,
            health: data.health,
            game_mode: data.game_mode,
            prev_game_mode: data.prev_game_mode,
            abilities: AbilitiesFile {
                invulnerable: data.abilities.invulnerable,
                flying: data.abilities.flying,
                may_fly: data.abilities.may_fly,
                instabuild: data.abilities.instabuild,
                may_build: data.abilities.may_build,
                flying_speed: data.abilities.flying_speed,
                walking_speed: data.abilities.walking_speed,
            },
            inventory: slots_to_file(&data.inventory)?,
            ender_chest: slots_to_file(&data.ender_chest)?,
            selected_slot: data.selected_slot,
            world: data.world.clone(),
            food_level: data.food_level,
            food_saturation_level: data.food_saturation_level,
            food_exhaustion_level: data.food_exhaustion_level,
            food_tick_timer: data.food_tick_timer,
            experience_level: data.experience_level,
            experience_progress: data.experience_progress,
            experience_total: data.experience_total,
            score: data.score,
            root_vehicle: data
                .root_vehicle
                .clone()
                .map(|root_vehicle| RootVehicleFile {
                    attach: root_vehicle.attach,
                    entity: root_vehicle.entity,
                }),
        })
    }

    pub(super) fn into_persistent(self) -> io::Result<PersistentPlayerData> {
        if self.data_version != PLAYER_DATA_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported player data payload version {}",
                    self.data_version
                ),
            ));
        }

        let inventory = slots_from_file(self.inventory)?;
        let ender_chest = slots_from_file(self.ender_chest)?;

        Ok(PersistentPlayerData {
            pos: self.pos,
            motion: self.motion,
            rotation: self.rotation,
            on_ground: self.on_ground,
            fall_flying: self.fall_flying,
            remaining_fire_ticks: self.remaining_fire_ticks,
            ticks_frozen: self.ticks_frozen,
            is_in_powder_snow: self.is_in_powder_snow,
            was_in_powder_snow: self.was_in_powder_snow,
            has_visual_fire: self.has_visual_fire,
            health: self.health,
            game_mode: self.game_mode,
            prev_game_mode: self.prev_game_mode,
            abilities: PersistentAbilities {
                invulnerable: self.abilities.invulnerable,
                flying: self.abilities.flying,
                may_fly: self.abilities.may_fly,
                instabuild: self.abilities.instabuild,
                may_build: self.abilities.may_build,
                flying_speed: self.abilities.flying_speed,
                walking_speed: self.abilities.walking_speed,
            },
            inventory,
            ender_chest,
            selected_slot: self.selected_slot,
            world: self.world,
            food_level: self.food_level,
            food_saturation_level: self.food_saturation_level,
            food_exhaustion_level: self.food_exhaustion_level,
            food_tick_timer: self.food_tick_timer,
            data_version: self.data_version,
            experience_level: self.experience_level,
            experience_progress: self.experience_progress,
            experience_total: self.experience_total,
            score: self.score,
            root_vehicle: self.root_vehicle.map(|root_vehicle| PersistentRootVehicle {
                attach: root_vehicle.attach,
                entity: root_vehicle.entity,
            }),
        })
    }
}

fn slots_to_file(slots: &[PersistentSlot]) -> io::Result<Vec<SlotFile>> {
    let mut file_slots = Vec::with_capacity(slots.len());
    for slot in slots {
        file_slots.push(SlotFile {
            slot: slot.slot,
            item_nbt: item_to_nbt_bytes(&slot.item)?,
        });
    }
    Ok(file_slots)
}

fn slots_from_file(slots: Vec<SlotFile>) -> io::Result<Vec<PersistentSlot>> {
    let mut persistent_slots = Vec::with_capacity(slots.len());
    for slot in slots {
        persistent_slots.push(PersistentSlot {
            slot: slot.slot,
            item: item_from_nbt_bytes(&slot.item_nbt)?,
        });
    }
    Ok(persistent_slots)
}

pub(super) fn item_to_nbt_bytes(item: &ItemStack) -> io::Result<Vec<u8>> {
    let NbtTag::Compound(compound) = item.clone().to_nbt_tag() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "item stack did not serialize to a compound",
        ));
    };
    let mut bytes = Vec::new();
    compound.write(&mut bytes);
    Ok(bytes)
}

fn item_from_nbt_bytes(bytes: &[u8]) -> io::Result<ItemStack> {
    let nbt = read_borrowed_compound(&mut Cursor::new(bytes)).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to parse item NBT: {e}"),
        )
    })?;
    let compound = simdnbt::borrow::NbtCompound::from(&nbt);
    ItemStack::from_borrowed_compound(&compound)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid item stack data"))
}

pub(super) fn encode_player_file(file: &PlayerDataFile) -> io::Result<Vec<u8>> {
    encode_file(
        PLAYER_MAGIC,
        PLAYER_STORAGE_VERSION,
        wincode::serialize(file),
    )
}

pub(super) fn decode_player_file(bytes: &[u8]) -> io::Result<PlayerDataFile> {
    let payload = decode_file(PLAYER_MAGIC, PLAYER_STORAGE_VERSION, bytes)?;
    wincode::deserialize(&payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
}
