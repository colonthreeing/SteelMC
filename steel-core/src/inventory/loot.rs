//! Loot-table helpers for filling item containers.

use glam::DVec3;
use rand::rngs::StdRng;
use rand::seq::SliceRandom as _;
use rand::{RngExt as _, SeedableRng as _};
use steel_registry::item_stack::ItemStack;
use steel_registry::loot_table::{LootContext, LootTable};

/// Fills empty slots from a loot table using vanilla's container placement
/// shape over Steel's current loot-table RNG contract.
pub(crate) fn fill_slots_from_loot_table(
    slots: &mut [ItemStack],
    loot_table: &LootTable,
    seed: i64,
    origin: Option<DVec3>,
    luck: f32,
) {
    let rng_seed = if seed == 0 {
        rand::random()
    } else {
        seed as u64
    };
    let mut rng = StdRng::seed_from_u64(rng_seed);
    let items = {
        let mut ctx = LootContext::new(&mut rng).with_luck(luck);
        if let Some(origin) = origin {
            ctx = ctx.with_origin(origin.x, origin.y, origin.z);
        }
        loot_table.get_random_items(&mut ctx)
    };
    fill_slots_from_items(slots, items, &mut rng);
}

fn fill_slots_from_items(
    slots: &mut [ItemStack],
    mut items: Vec<ItemStack>,
    rng: &mut impl rand::Rng,
) {
    let mut available_slots = available_empty_slots(slots, rng);
    shuffle_and_split_items(&mut items, available_slots.len(), rng);

    for item in items {
        let Some(slot) = available_slots.pop() else {
            log::warn!("Tried to over-fill a container");
            return;
        };
        slots[slot] = item;
    }
}

fn available_empty_slots(slots: &[ItemStack], rng: &mut impl rand::Rng) -> Vec<usize> {
    let mut available_slots = slots
        .iter()
        .enumerate()
        .filter_map(|(slot, item)| item.is_empty().then_some(slot))
        .collect::<Vec<_>>();
    available_slots.shuffle(rng);
    available_slots
}

fn shuffle_and_split_items(
    result: &mut Vec<ItemStack>,
    available_slots: usize,
    rng: &mut impl rand::Rng,
) {
    let mut splittable_items = Vec::new();
    result.retain_mut(|item| {
        if item.is_empty() {
            return false;
        }
        if item.count() > 1 {
            splittable_items.push(item.clone());
            return false;
        }
        true
    });

    while available_slots
        .saturating_sub(result.len())
        .saturating_sub(splittable_items.len())
        > 0
        && !splittable_items.is_empty()
    {
        let index = rng.random_range(0..splittable_items.len());
        let mut item = splittable_items.remove(index);
        let split_count = rng.random_range(1..=item.count() / 2);
        let split = item.split(split_count);

        if item.count() > 1 && rng.random::<bool>() {
            splittable_items.push(item);
        } else {
            result.push(item);
        }

        if split.count() > 1 && rng.random::<bool>() {
            splittable_items.push(split);
        } else {
            result.push(split);
        }
    }

    result.extend(splittable_items);
    result.shuffle(rng);
}

#[cfg(test)]
mod tests {
    use rand::SeedableRng as _;
    use steel_registry::item_stack::ItemStack;
    use steel_registry::vanilla_items;

    use super::fill_slots_from_items;

    #[test]
    fn fill_slots_from_items_preserves_existing_slots_and_splits_stacks() {
        let mut rng = rand::rngs::StdRng::seed_from_u64(123);
        let existing = ItemStack::with_count(&vanilla_items::ITEMS.stone, 1);
        let mut slots = vec![ItemStack::empty(); 4];
        slots[0] = existing.clone();

        fill_slots_from_items(
            &mut slots,
            vec![ItemStack::with_count(&vanilla_items::ITEMS.oak_log, 8)],
            &mut rng,
        );

        assert_eq!(slots[0], existing);
        assert_eq!(
            slots
                .iter()
                .filter(|item| item.item() == &vanilla_items::ITEMS.oak_log)
                .map(ItemStack::count)
                .sum::<i32>(),
            8
        );
        assert!(
            slots
                .iter()
                .filter(|item| item.item() == &vanilla_items::ITEMS.oak_log)
                .count()
                > 1
        );
    }
}
