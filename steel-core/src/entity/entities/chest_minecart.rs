//! Chest minecart state needed by structure generation and persistence.

use std::str::FromStr;
use std::sync::Weak;

use glam::DVec3;
use simdnbt::ToNbtTag;
use simdnbt::borrow::NbtCompound as BorrowedNbtCompoundView;
use simdnbt::owned::{NbtCompound, NbtList, NbtTag};
use steel_macros::entity_behavior;
use steel_registry::entity_type::EntityTypeRef;
use steel_registry::item_stack::ItemStack;
use steel_utils::Identifier;
use steel_utils::locks::SyncMutex;

use crate::entity::{Entity, EntityBase, EntityBaseLoad, EntityCommandItemSlotResult};
use crate::world::World;

const CHEST_MINECART_CONTAINER_SIZE: usize = 36;

/// Chest minecart entity state used by mineshaft generation.
///
/// Steel does not yet implement minecart movement or container interaction, but
/// the entity preserves vanilla placement, loot-table state, and concrete
/// container item state for persistence and command access.
#[entity_behavior(class = "MinecartChest")]
pub struct ChestMinecartEntity {
    base: EntityBase,
    entity_type: EntityTypeRef,
    state: SyncMutex<ChestMinecartState>,
}

#[derive(Debug, Clone, PartialEq)]
struct ChestMinecartState {
    first_tick: bool,
    items: Vec<ItemStack>,
    loot_table: Option<Identifier>,
    loot_table_seed: i64,
}

impl ChestMinecartState {
    fn new(first_tick: bool) -> Self {
        Self {
            first_tick,
            items: vec![ItemStack::empty(); CHEST_MINECART_CONTAINER_SIZE],
            loot_table: None,
            loot_table_seed: 0,
        }
    }
}

impl ChestMinecartEntity {
    /// Creates a new chest minecart entity.
    #[must_use]
    pub fn new(entity_type: EntityTypeRef, id: i32, position: DVec3, world: Weak<World>) -> Self {
        Self {
            base: EntityBase::new(id, position, entity_type.dimensions, world),
            entity_type,
            state: SyncMutex::new(ChestMinecartState::new(true)),
        }
    }

    /// Creates a chest minecart entity from saved data.
    #[must_use]
    pub fn from_saved(entity_type: EntityTypeRef, load: EntityBaseLoad) -> Self {
        Self {
            base: EntityBase::from_load(load, entity_type.dimensions),
            entity_type,
            state: SyncMutex::new(ChestMinecartState::new(false)),
        }
    }

    /// Sets the deferred loot table used when the container is first opened.
    pub fn set_loot_table(&self, loot_table: Identifier, seed: i64) {
        let mut state = self.state.lock();
        state.loot_table = Some(loot_table);
        state.loot_table_seed = seed;
    }

    const fn nbt_bool(value: bool) -> i8 {
        if value { 1 } else { 0 }
    }

    fn save_items(state: &ChestMinecartState, nbt: &mut NbtCompound) {
        let mut items = Vec::new();
        for (slot, item) in state.items.iter().enumerate() {
            if !item.is_empty()
                && let NbtTag::Compound(mut item_nbt) = item.clone().to_nbt_tag()
            {
                item_nbt.insert("Slot", slot as i8);
                items.push(item_nbt);
            }
        }
        nbt.insert("Items", NbtList::Compound(items));
    }

    fn load_items(state: &mut ChestMinecartState, nbt: BorrowedNbtCompoundView<'_, '_>) {
        state.items.fill(ItemStack::empty());
        if let Some(items_list) = nbt.list("Items")
            && let Some(compounds) = items_list.compounds()
        {
            for compound in compounds {
                let Some(slot) = compound
                    .byte("Slot")
                    .and_then(|slot| usize::try_from(slot).ok())
                    .filter(|slot| *slot < CHEST_MINECART_CONTAINER_SIZE)
                else {
                    continue;
                };
                if let Some(item) = ItemStack::from_borrowed_compound(&compound) {
                    state.items[slot] = item;
                }
            }
        }
    }
}

impl Entity for ChestMinecartEntity {
    fn base(&self) -> &EntityBase {
        &self.base
    }

    fn entity_type(&self) -> EntityTypeRef {
        self.entity_type
    }

    fn is_pickable(&self) -> bool {
        !self.is_removed()
    }

    fn is_pushable(&self) -> bool {
        true
    }

    fn blocks_building(&self) -> bool {
        true
    }

    fn save_additional(&self, nbt: &mut NbtCompound) {
        nbt.insert("FlippedRotation", Self::nbt_bool(false));
        let state = self.state.lock();
        nbt.insert("HasTicked", Self::nbt_bool(state.first_tick));

        if let Some(loot_table) = state.loot_table.as_ref() {
            nbt.insert("LootTable", loot_table.to_string());
            if state.loot_table_seed != 0 {
                nbt.insert("LootTableSeed", NbtTag::Long(state.loot_table_seed));
            }
        } else {
            Self::save_items(&state, nbt);
        }
    }

    fn load_additional(&self, nbt: BorrowedNbtCompoundView<'_, '_>) {
        let loot_table = nbt
            .string("LootTable")
            .and_then(|value| Identifier::from_str(&value.to_string()).ok());
        let mut state = self.state.lock();
        if let Some(first_tick) = nbt.byte("HasTicked") {
            state.first_tick = first_tick != 0;
        }
        state.loot_table = loot_table;
        state.loot_table_seed = nbt.long("LootTableSeed").unwrap_or(0);
        if state.loot_table.is_none() {
            Self::load_items(&mut state, nbt);
        } else {
            state.items.fill(ItemStack::empty());
        }
    }

    fn with_command_item_slot(
        &self,
        slot: i32,
        visitor: &mut dyn FnMut(&ItemStack),
    ) -> EntityCommandItemSlotResult {
        let Ok(slot) = usize::try_from(slot) else {
            return EntityCommandItemSlotResult::Missing;
        };
        if slot >= CHEST_MINECART_CONTAINER_SIZE {
            return EntityCommandItemSlotResult::Missing;
        }

        let state = self.state.lock();
        if state.loot_table.is_some() {
            return EntityCommandItemSlotResult::Unsupported;
        }

        visitor(&state.items[slot]);
        EntityCommandItemSlotResult::Found
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use simdnbt::borrow::read_compound as read_borrowed_compound;
    use steel_registry::test_support::init_test_registry;
    use steel_registry::vanilla_entities;
    use steel_registry::vanilla_items;

    fn load_owned_nbt(minecart: &ChestMinecartEntity, nbt: &NbtCompound) {
        let mut bytes = Vec::new();
        nbt.write(&mut bytes);
        let borrowed = read_borrowed_compound(&mut Cursor::new(&bytes))
            .unwrap_or_else(|error| panic!("test nbt should reborrow: {error}"));
        minecart.load_additional((&borrowed).into());
    }

    fn item_stack_nbt(slot: i8, item: &ItemStack) -> NbtCompound {
        let NbtTag::Compound(mut item_nbt) = item.clone().to_nbt_tag() else {
            return NbtCompound::new();
        };
        item_nbt.insert("Slot", slot);
        item_nbt
    }

    #[test]
    fn chest_minecart_saves_structure_loot_table_state() {
        let minecart = ChestMinecartEntity::new(
            &vanilla_entities::CHEST_MINECART,
            1,
            DVec3::new(1.5, 2.5, 3.5),
            Weak::new(),
        );
        minecart.set_loot_table(
            Identifier::new_static("minecraft", "chests/abandoned_mineshaft"),
            42,
        );

        let mut nbt = NbtCompound::new();
        minecart.save_additional(&mut nbt);

        assert_eq!(
            nbt.string("LootTable").map(ToString::to_string),
            Some("minecraft:chests/abandoned_mineshaft".to_owned())
        );
        assert_eq!(nbt.long("LootTableSeed"), Some(42));
        assert_eq!(nbt.byte("HasTicked"), Some(1));
        assert_eq!(nbt.byte("FlippedRotation"), Some(0));
        assert!(nbt.get("Items").is_none());
    }

    #[test]
    fn chest_minecart_persists_concrete_container_items() {
        init_test_registry();
        let minecart = ChestMinecartEntity::new(
            &vanilla_entities::CHEST_MINECART,
            1,
            DVec3::new(1.5, 2.5, 3.5),
            Weak::new(),
        );
        let stack = ItemStack::with_count(&vanilla_items::ITEMS.stone, 4);
        let mut source_nbt = NbtCompound::new();
        source_nbt.insert("Items", NbtList::Compound(vec![item_stack_nbt(5, &stack)]));
        load_owned_nbt(&minecart, &source_nbt);

        let mut saved_nbt = NbtCompound::new();
        minecart.save_additional(&mut saved_nbt);

        assert!(saved_nbt.get("LootTable").is_none());
        let Some(NbtTag::List(NbtList::Compound(items))) = saved_nbt.get("Items") else {
            panic!("chest minecart should save concrete items list");
        };
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].byte("Slot"), Some(5));
        assert_eq!(
            items[0].string("id").map(ToString::to_string),
            Some("minecraft:stone".to_owned())
        );
        assert_eq!(items[0].int("count"), Some(4));

        let loaded = ChestMinecartEntity::new(
            &vanilla_entities::CHEST_MINECART,
            2,
            DVec3::new(1.5, 2.5, 3.5),
            Weak::new(),
        );
        load_owned_nbt(&loaded, &saved_nbt);
        let result = loaded.with_command_item_slot(5, &mut |item| {
            assert_eq!(item.item(), &vanilla_items::ITEMS.stone);
            assert_eq!(item.count(), 4);
        });
        assert_eq!(result, EntityCommandItemSlotResult::Found);
    }

    #[test]
    fn chest_minecart_exposes_container_command_slots() {
        init_test_registry();
        let minecart = ChestMinecartEntity::new(
            &vanilla_entities::CHEST_MINECART,
            1,
            DVec3::new(1.5, 2.5, 3.5),
            Weak::new(),
        );
        let stack = ItemStack::with_count(&vanilla_items::ITEMS.oak_log, 2);
        let mut source_nbt = NbtCompound::new();
        source_nbt.insert("Items", NbtList::Compound(vec![item_stack_nbt(35, &stack)]));
        load_owned_nbt(&minecart, &source_nbt);

        let mut visited = false;
        let result = minecart.with_command_item_slot(35, &mut |item| {
            visited = true;
            assert_eq!(item.item(), &vanilla_items::ITEMS.oak_log);
            assert_eq!(item.count(), 2);
        });
        assert_eq!(result, EntityCommandItemSlotResult::Found);
        assert!(visited);

        let result = minecart.with_command_item_slot(36, &mut |_| {});
        assert_eq!(result, EntityCommandItemSlotResult::Missing);
        let result = minecart.with_command_item_slot(-1, &mut |_| {});
        assert_eq!(result, EntityCommandItemSlotResult::Missing);
    }

    #[test]
    fn chest_minecart_defers_pending_loot_table_slots() {
        let minecart = ChestMinecartEntity::new(
            &vanilla_entities::CHEST_MINECART,
            1,
            DVec3::new(1.5, 2.5, 3.5),
            Weak::new(),
        );
        minecart.set_loot_table(
            Identifier::new_static("minecraft", "chests/abandoned_mineshaft"),
            42,
        );

        let result = minecart.with_command_item_slot(0, &mut |_| {
            panic!("pending loot table should not expose an item before loot unpacking exists");
        });
        assert_eq!(result, EntityCommandItemSlotResult::Unsupported);
    }

    #[test]
    fn chest_minecart_is_pickable_and_pushable_like_vanilla() {
        let minecart = ChestMinecartEntity::new(
            &vanilla_entities::CHEST_MINECART,
            1,
            DVec3::new(1.5, 2.5, 3.5),
            Weak::new(),
        );

        assert!(minecart.is_pickable());
        assert!(minecart.is_pushable());
        assert!(minecart.blocks_building());
    }
}
