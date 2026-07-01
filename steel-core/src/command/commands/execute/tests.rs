use std::sync::{Arc, Weak};

use glam::DVec3;
use simdnbt::owned::NbtTag;
use steel_registry::{
    entity_type::EntityTypeRef, item_stack::ItemStack, test_support::init_test_registry,
    vanilla_entities, vanilla_items,
};
use steel_utils::{Identifier, nbt::parse_nbt_path, translations};
use text_components::content::Content;

use crate::command::error::CommandError;
use crate::command::graph::{
    CommandGraph, ItemPredicateArgumentValue, ItemPredicateTarget, ItemSlotRangeArgumentValue,
    ParsedArgument, ParsedArguments,
};
use crate::command::requirement::{
    CommandInputContext, CommandSourceKind, PermissionExpr, RequirementContext,
};
use crate::command::storage::CommandStorage;
use crate::entity::{Entity, EntityBase, entities::ItemEntity};
use crate::scoreboard::{ScoreHolder, Scoreboard};

struct TestContext;

struct StoreTestEntity {
    base: EntityBase,
}

impl StoreTestEntity {
    fn shared(id: i32, position: DVec3) -> Arc<Self> {
        Arc::new(Self {
            base: EntityBase::new(id, position, vanilla_entities::ITEM.dimensions, Weak::new()),
        })
    }
}

impl Entity for StoreTestEntity {
    fn base(&self) -> &EntityBase {
        &self.base
    }

    fn entity_type(&self) -> EntityTypeRef {
        &vanilla_entities::ITEM
    }
}

impl RequirementContext for TestContext {
    fn source_kind(&self) -> CommandSourceKind {
        CommandSourceKind::Player
    }

    fn has_permission(&self, _permission: &PermissionExpr) -> bool {
        false
    }
}

impl CommandInputContext for TestContext {
    fn position(&self) -> Option<DVec3> {
        Some(DVec3::ZERO)
    }
}

fn graph() -> CommandGraph {
    CommandGraph::new()
        .with_root(super::command())
        .expect("execute command registers")
}

fn stone_predicate() -> ItemPredicateArgumentValue {
    ItemPredicateArgumentValue::new(
        ItemPredicateTarget::Item(&vanilla_items::ITEMS.stone),
        Vec::new(),
    )
}

fn command_failed_translation_key(error: CommandError) -> String {
    let CommandError::CommandFailed(message) = error else {
        panic!("error should be a command failure");
    };
    let Content::Translate(message) = &message.content else {
        panic!("command failure should be translated");
    };
    message.key.to_string()
}

fn entity_item_arguments(
    entity: Arc<dyn Entity>,
    slots: ItemSlotRangeArgumentValue,
) -> ParsedArguments {
    let mut arguments = ParsedArguments::default();
    arguments.insert("entities", ParsedArgument::Entities(vec![entity]));
    arguments.insert("slots", ParsedArgument::ItemSlots(slots));
    arguments.insert(
        "item_predicate",
        ParsedArgument::ItemPredicate(stone_predicate()),
    );
    arguments
}

#[test]
fn source_transform_parse_shapes_match_vanilla_non_function_forms() {
    init_test_registry();
    let graph = graph();
    let context = TestContext;
    let cases: &[(&str, &[&str])] = &[
        ("execute as Steve run seed", &["execute", "as", "targets"]),
        ("execute at Steve run seed", &["execute", "at", "targets"]),
        (
            "execute positioned 1 2 3 run seed",
            &["execute", "positioned", "pos"],
        ),
        (
            "execute positioned as Steve run seed",
            &["execute", "positioned", "as", "targets"],
        ),
        (
            "execute rotated 90 0 run seed",
            &["execute", "rotated", "rot"],
        ),
        (
            "execute rotated as Steve run seed",
            &["execute", "rotated", "as", "targets"],
        ),
        (
            "execute facing 1 2 3 run seed",
            &["execute", "facing", "pos"],
        ),
        (
            "execute facing entity Steve eyes run seed",
            &["execute", "facing", "entity", "targets", "anchor"],
        ),
        ("execute align xyz run seed", &["execute", "align", "axes"]),
        (
            "execute anchored eyes run seed",
            &["execute", "anchored", "anchor"],
        ),
        (
            "execute summon pig run seed",
            &["execute", "summon", "entity"],
        ),
    ];

    for (command, expected_path) in cases {
        let parsed = graph.parse(command, &context).unwrap_or_else(|_| {
            panic!("source transform parses: {command}");
        });
        assert_eq!(parsed.path(), *expected_path, "{command}");
    }
}

#[test]
fn loaded_condition_parses_direct_and_redirect_forms() {
    let graph = graph();
    let context = TestContext;

    let direct = graph
        .parse("execute if loaded 0 64 0", &context)
        .expect("direct loaded conditional parses");
    assert_eq!(direct.path(), ["execute", "if", "loaded", "pos"]);

    let redirected = graph
        .parse("execute unless loaded 0 64 0 run seed", &context)
        .expect("redirected loaded conditional parses");
    assert_eq!(redirected.path(), ["execute", "unless", "loaded", "pos"]);
}

#[test]
fn stopwatch_condition_parses_direct_and_redirect_forms() {
    let graph = graph();
    let context = TestContext;

    let direct = graph
        .parse("execute if stopwatch minecraft:test 0.5..1.5", &context)
        .expect("direct stopwatch conditional parses");
    assert_eq!(direct.path(), ["execute", "if", "stopwatch", "id", "range"]);

    let redirected = graph
        .parse("execute unless stopwatch test ..5 run seed", &context)
        .expect("redirected stopwatch conditional parses");
    assert_eq!(
        redirected.path(),
        ["execute", "unless", "stopwatch", "id", "range"]
    );
}

#[test]
fn predicate_condition_parses_direct_and_redirect_forms() {
    let graph = graph();
    let context = TestContext;

    let direct = graph
        .parse(
            "execute if predicate {condition:\"minecraft:killed_by_player\"}",
            &context,
        )
        .expect("direct predicate conditional parses");
    assert_eq!(direct.path(), ["execute", "if", "predicate", "predicate"]);

    let redirected = graph
        .parse("execute unless predicate test run seed", &context)
        .expect("redirected predicate conditional parses");
    assert_eq!(
        redirected.path(),
        ["execute", "unless", "predicate", "predicate"]
    );
}

#[test]
fn function_condition_matches_vanilla_redirect_only_shape() {
    let graph = graph();
    let context = TestContext;

    let redirected = graph
        .parse("execute if function test:gate run seed", &context)
        .expect("redirected function conditional parses");
    assert_eq!(redirected.path(), ["execute", "if", "function", "name"]);

    assert!(
        graph
            .parse("execute unless function #test:gates", &context)
            .is_err(),
        "function conditions require a redirected command tail"
    );
}

#[test]
fn bossbar_store_parses_value_and_max_targets() {
    let graph = graph();
    let context = TestContext;

    let value = graph
        .parse(
            "execute store result bossbar minecraft:test value run seed",
            &context,
        )
        .expect("bossbar value store parses");
    assert_eq!(
        value.path(),
        ["execute", "store", "result", "bossbar", "id", "value"]
    );

    let max = graph
        .parse("execute store success bossbar test max run seed", &context)
        .expect("bossbar max store parses");
    assert_eq!(
        max.path(),
        ["execute", "store", "success", "bossbar", "id", "max"]
    );
}

#[test]
fn biome_condition_parses_direct_and_redirect_forms() {
    init_test_registry();
    let graph = graph();
    let context = TestContext;

    let direct = graph
        .parse("execute if biome 0 64 0 plains", &context)
        .expect("direct biome conditional parses");
    assert_eq!(direct.path(), ["execute", "if", "biome", "pos", "biome"]);

    let redirected = graph
        .parse("execute unless biome 0 64 0 #is_overworld run seed", &context)
        .expect("redirected biome conditional parses");
    assert_eq!(
        redirected.path(),
        ["execute", "unless", "biome", "pos", "biome"]
    );
}

#[test]
fn block_condition_parses_direct_and_redirect_forms() {
    init_test_registry();
    let graph = graph();
    let context = TestContext;

    let direct = graph
        .parse("execute if block 0 64 0 stone", &context)
        .expect("direct block conditional parses");
    assert_eq!(direct.path(), ["execute", "if", "block", "pos", "block"]);

    let redirected = graph
        .parse(
            "execute unless block 0 64 0 oak_log[axis=y]{id:'minecraft:barrel'} run seed",
            &context,
        )
        .expect("redirected block conditional parses");
    assert_eq!(
        redirected.path(),
        ["execute", "unless", "block", "pos", "block"]
    );
}

#[test]
fn entity_condition_parses_direct_and_redirect_forms() {
    let graph = graph();
    let context = TestContext;

    let direct = graph
        .parse("execute if entity Steve", &context)
        .expect("direct entity conditional parses");
    assert_eq!(direct.path(), ["execute", "if", "entity", "entities"]);

    let redirected = graph
        .parse("execute unless entity Steve run seed", &context)
        .expect("redirected entity conditional parses");
    assert_eq!(
        redirected.path(),
        ["execute", "unless", "entity", "entities"]
    );
}

#[test]
fn data_block_condition_parses_direct_and_redirect_forms() {
    let graph = graph();
    let context = TestContext;

    let direct = graph
        .parse(
            "execute if data block 0 64 0 Items[{id:\"minecraft:stone\"}].Count",
            &context,
        )
        .expect("direct data block conditional parses");
    assert_eq!(
        direct.path(),
        ["execute", "if", "data", "block", "pos", "path"]
    );

    let redirected = graph
        .parse("execute unless data block 0 64 0 Items[] run seed", &context)
        .expect("redirected data block conditional parses");
    assert_eq!(
        redirected.path(),
        ["execute", "unless", "data", "block", "pos", "path"]
    );
}

#[test]
fn data_entity_condition_parses_direct_and_redirect_forms() {
    let graph = graph();
    let context = TestContext;

    let direct = graph
        .parse("execute if data entity Steve Air", &context)
        .expect("direct data entity conditional parses");
    assert_eq!(
        direct.path(),
        ["execute", "if", "data", "entity", "source", "path"]
    );

    let redirected = graph
        .parse("execute unless data entity Steve Air run seed", &context)
        .expect("redirected data entity conditional parses");
    assert_eq!(
        redirected.path(),
        ["execute", "unless", "data", "entity", "source", "path"]
    );
}

#[test]
fn data_storage_condition_parses_direct_and_redirect_forms() {
    let graph = graph();
    let context = TestContext;

    let direct = graph
        .parse("execute if data storage steel:data value", &context)
        .expect("direct data storage conditional parses");
    assert_eq!(
        direct.path(),
        ["execute", "if", "data", "storage", "source", "path"]
    );

    let redirected = graph
        .parse("execute unless data storage steel:data value run seed", &context)
        .expect("redirected data storage conditional parses");
    assert_eq!(
        redirected.path(),
        ["execute", "unless", "data", "storage", "source", "path"]
    );
}

#[test]
fn data_condition_suggests_entity_accessor() {
    let graph = graph();
    let context = TestContext;

    let suggestions = graph
        .suggest("execute if data ", &context)
        .expect("data condition suggestions");
    let suggestions = suggestions
        .suggestions
        .iter()
        .map(|suggestion| suggestion.text.as_str())
        .collect::<Vec<_>>();

    assert!(suggestions.contains(&"entity"));
}

#[test]
fn items_entity_condition_parses_direct_and_redirect_forms() {
    init_test_registry();
    let graph = graph();
    let context = TestContext;

    let direct = graph
        .parse("execute if items entity Steve contents stone", &context)
        .expect("direct items entity conditional parses");
    assert_eq!(
        direct.path(),
        [
            "execute",
            "if",
            "items",
            "entity",
            "entities",
            "slots",
            "item_predicate"
        ]
    );

    let redirected = graph
        .parse(
            "execute unless items entity Steve contents #logs[count={min:2}] run seed",
            &context,
        )
        .expect("redirected items entity conditional parses");
    assert_eq!(
        redirected.path(),
        [
            "execute",
            "unless",
            "items",
            "entity",
            "entities",
            "slots",
            "item_predicate"
        ]
    );
}

#[test]
fn items_block_condition_parses_direct_and_redirect_forms() {
    init_test_registry();
    let graph = graph();
    let context = TestContext;

    let direct = graph
        .parse("execute if items block 0 64 0 container.* stone", &context)
        .expect("direct items block conditional parses");
    assert_eq!(
        direct.path(),
        [
            "execute",
            "if",
            "items",
            "block",
            "pos",
            "slots",
            "item_predicate"
        ]
    );

    let redirected = graph
        .parse(
            "execute unless items block 0 64 0 container.0 #logs[count={min:2}] run seed",
            &context,
        )
        .expect("redirected items block conditional parses");
    assert_eq!(
        redirected.path(),
        [
            "execute",
            "unless",
            "items",
            "block",
            "pos",
            "slots",
            "item_predicate"
        ]
    );
}

#[test]
fn items_condition_suggests_entity_source() {
    init_test_registry();
    let graph = graph();
    let context = TestContext;

    let suggestions = graph
        .suggest("execute if items ", &context)
        .expect("items condition suggestions");
    let suggestions = suggestions
        .suggestions
        .iter()
        .map(|suggestion| suggestion.text.as_str())
        .collect::<Vec<_>>();

    assert!(suggestions.contains(&"entity"));
    assert!(suggestions.contains(&"block"));
}

#[test]
fn items_entity_condition_counts_item_entity_contents() {
    init_test_registry();
    let item = ItemStack::with_count(&vanilla_items::ITEMS.stone, 4);
    let entity: Arc<dyn Entity> = Arc::new(ItemEntity::with_item(
        &vanilla_entities::ITEM,
        1,
        DVec3::ZERO,
        item,
        Weak::new(),
    ));
    let arguments = entity_item_arguments(
        entity,
        ItemSlotRangeArgumentValue::new("contents", vec![0]),
    );

    match super::condition::entity_items_match_count(&TestContext, &arguments) {
        Ok(count) => assert_eq!(count, 4),
        Err(_) => panic!("item entity contents count should succeed"),
    }
}

#[test]
fn items_entity_condition_ignores_missing_slots() {
    init_test_registry();
    let entity: Arc<dyn Entity> = StoreTestEntity::shared(1, DVec3::ZERO);
    let arguments = entity_item_arguments(
        entity,
        ItemSlotRangeArgumentValue::new("contents", vec![0]),
    );

    match super::condition::entity_items_match_count(&TestContext, &arguments) {
        Ok(count) => assert_eq!(count, 0),
        Err(_) => panic!("missing entity slots should count as zero"),
    }
}

#[test]
fn score_condition_parses_comparison_and_range_forms() {
    let graph = graph();
    let context = TestContext;

    let comparison = graph
        .parse("execute if score Steve kills = Alex kills", &context)
        .expect("score comparison conditional parses");
    assert_eq!(
        comparison.path(),
        [
            "execute",
            "if",
            "score",
            "target",
            "targetObjective",
            "=",
            "source",
            "sourceObjective"
        ]
    );

    let range = graph
        .parse("execute unless score Steve kills matches 1.. run seed", &context)
        .expect("score range conditional parses");
    assert_eq!(
        range.path(),
        [
            "execute",
            "unless",
            "score",
            "target",
            "targetObjective",
            "matches",
            "range"
        ]
    );
}

#[test]
fn store_score_parses_redirect_form() {
    let graph = graph();
    let context = TestContext;

    let parsed = graph
        .parse("execute store result score Steve kills run seed", &context)
        .expect("store score parses");
    assert_eq!(
        parsed.path(),
        ["execute", "store", "result", "score", "targets", "objective"]
    );
}

#[test]
fn store_block_data_parses_redirect_form() {
    let graph = graph();
    let context = TestContext;

    let parsed = graph
        .parse(
            "execute store result block 0 64 0 Items[0].Count int 1 run seed",
            &context,
        )
        .expect("store block data parses");
    assert_eq!(
        parsed.path(),
        [
            "execute",
            "store",
            "result",
            "block",
            "targetPos",
            "path",
            "int",
            "scale"
        ]
    );
}

#[test]
fn store_storage_data_parses_redirect_form() {
    let graph = graph();
    let context = TestContext;

    let parsed = graph
        .parse(
            "execute store result storage steel:data value int 1 run seed",
            &context,
        )
        .expect("store storage data parses");
    assert_eq!(
        parsed.path(),
        [
            "execute", "store", "result", "storage", "target", "path", "int", "scale"
        ]
    );
}

#[test]
fn store_entity_data_parses_redirect_form() {
    let graph = graph();
    let context = TestContext;

    let parsed = graph
        .parse(
            "execute store result entity Steve Air int 1 run seed",
            &context,
        )
        .expect("store entity data parses");
    assert_eq!(
        parsed.path(),
        [
            "execute", "store", "result", "entity", "target", "path", "int", "scale"
        ]
    );
}

#[test]
fn store_data_type_converts_scaled_value() {
    assert_eq!(super::store::StoreDataType::Int.tag(3, 2.5), NbtTag::Int(7));
    assert_eq!(
        super::store::StoreDataType::Long.tag(-3, 2.5),
        NbtTag::Long(-7)
    );
    assert_eq!(
        super::store::StoreDataType::Byte.tag(258, 1.0),
        NbtTag::Byte(2)
    );
    assert_eq!(
        super::store::StoreDataType::Short.tag(65_538, 1.0),
        NbtTag::Short(2)
    );
    assert_eq!(
        super::store::StoreDataType::Float.tag(3, 0.5),
        NbtTag::Float(1.5)
    );
    assert_eq!(
        super::store::StoreDataType::Double.tag(3, 0.5),
        NbtTag::Double(1.5)
    );
}

#[test]
fn store_storage_data_value_updates_command_storage() {
    let storage = CommandStorage::new();
    let key = Identifier::new_static("steel", "data");
    let path = parse_nbt_path("value").expect("path parses");

    super::store::store_storage_data_value(&storage, &key, &path, NbtTag::Int(7))
        .expect("storage write succeeds");

    let data = storage.get(&key);
    assert_eq!(data.get("value"), Some(&NbtTag::Int(7)));
}

#[test]
fn store_entity_data_value_updates_entity_data() {
    init_test_registry();

    let entity = StoreTestEntity::shared(1, DVec3::ZERO);
    let path = parse_nbt_path("Air").expect("path parses");

    super::store::store_entity_data_value(entity.as_ref(), &path, NbtTag::Int(123))
        .expect("entity data updates");

    assert_eq!(entity.air_supply(), 123);
}

#[test]
fn score_comparison_requires_both_scores() {
    let scoreboard = Scoreboard::new();
    let kills = scoreboard
        .add_objective("kills")
        .expect("objective should be added");
    let steve = ScoreHolder::new("Steve");
    let alex = ScoreHolder::new("Alex");

    scoreboard
        .set_score(&steve, &kills, 3)
        .expect("score should be writable");
    assert!(!super::condition::compare_scores(
        &scoreboard,
        &steve,
        &kills,
        &alex,
        &kills,
        super::condition::ScoreComparison::Greater,
    ));

    scoreboard
        .set_score(&alex, &kills, 2)
        .expect("score should be writable");
    assert!(super::condition::compare_scores(
        &scoreboard,
        &steve,
        &kills,
        &alex,
        &kills,
        super::condition::ScoreComparison::Greater,
    ));
}

#[test]
fn store_score_value_writes_all_holders() {
    let scoreboard = Scoreboard::new();
    let objective = scoreboard
        .add_objective("result")
        .expect("objective should be added");
    let holders = [ScoreHolder::new("Steve"), ScoreHolder::new("Alex")];

    super::store::store_score_value(&scoreboard, &holders, &objective, 11)
        .expect("store should write scores");

    assert_eq!(scoreboard.score(&holders[0], &objective), Some(11));
    assert_eq!(scoreboard.score(&holders[1], &objective), Some(11));
}

#[test]
fn empty_score_holder_wildcard_uses_vanilla_error() {
    assert_eq!(
        command_failed_translation_key(super::empty_score_holder_wildcard()),
        translations::ARGUMENT_SCORE_HOLDER_EMPTY.0
    );
}

#[test]
fn positioned_over_parses_redirect_form() {
    let graph = graph();
    let context = TestContext;

    let parsed = graph
        .parse("execute positioned over motion_blocking run seed", &context)
        .expect("positioned over parses");
    assert_eq!(parsed.path(), ["execute", "positioned", "over", "heightmap"]);
}

#[test]
fn blocks_condition_parses_direct_and_redirect_forms() {
    let graph = graph();
    let context = TestContext;

    let direct = graph
        .parse("execute if blocks 0 64 0 1 64 1 10 64 10 all", &context)
        .expect("direct blocks conditional parses");
    assert_eq!(
        direct.path(),
        [
            "execute",
            "if",
            "blocks",
            "start",
            "end",
            "destination",
            "all"
        ]
    );

    let redirected = graph
        .parse("execute unless blocks 0 64 0 1 64 1 10 64 10 masked run seed", &context)
        .expect("redirected blocks conditional parses");
    assert_eq!(
        redirected.path(),
        [
            "execute",
            "unless",
            "blocks",
            "start",
            "end",
            "destination",
            "masked"
        ]
    );
}

#[test]
fn on_relations_parse_vanilla_relation_names() {
    let graph = graph();
    let context = TestContext;

    for relation in [
        "owner",
        "leasher",
        "target",
        "attacker",
        "vehicle",
        "controller",
        "origin",
        "passengers",
    ] {
        let parsed = graph
            .parse(&format!("execute on {relation} run seed"), &context)
            .expect("relation redirect parses");
        assert_eq!(parsed.path(), ["execute", "on", relation]);
    }
}
