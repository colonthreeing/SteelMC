//! Handler for the "execute" command.
//!
//! Bossbar store targets, predicates, functions, item predicates, and stopwatch
//! predicates are not registered here yet because their backing foundations are
//! not implemented in Steel's command/runtime layer.

use std::{borrow::Cow, sync::Arc};

use glam::DVec3;
use simdnbt::owned::NbtCompound;
use steel_registry::entity_type::EntityTypeRef;
use steel_utils::{
    BlockPos, Identifier,
    nbt::NbtPath,
    translations,
};
use text_components::TextComponent;
use text_components::translation::TranslatedMessage;

use crate::chunk::heightmap::HeightmapType;
use crate::command::CommandRegistrationSpec;
use crate::command::context::{CommandContext, EntityAnchor};
use crate::command::error::CommandError;
use crate::command::graph::{
    BiomeArgumentValue, BlockPredicateArgumentValue, CommandNodeBuilder, CommandRedirectTarget,
    CommandResult, IntRangeArgumentValue, ParsedArguments, ScoreHolderArgumentValue,
    ScoreboardObjectiveName, literal,
};
use crate::entity::SharedEntity;
use crate::scoreboard::{ScoreHolder, ScoreboardObjective};
use crate::world::World;

#[path = "execute/condition.rs"]
mod condition;
#[path = "execute/store.rs"]
mod store;
#[path = "execute/parsers.rs"]
mod execute_parsers;
#[path = "execute/source.rs"]
mod source;

use self::execute_parsers::{AxesParser, StorageKeyParser};

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Handler for the "execute" command.
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
    literal("execute")
        .then(literal("run").redirects(CommandRedirectTarget::All, |_, _| {
            Ok(CommandResult::success())
        }))
        .then(condition::conditionals("if", true))
        .then(condition::conditionals("unless", false))
        .then(source::as_operation())
        .then(source::at_operation())
        .then(
            literal("store")
                .then(store::target("result", true))
                .then(store::target("success", false)),
        )
        .then(source::positioned_operation())
        .then(source::rotated_operation())
        .then(source::facing_operation())
        .then(source::align_operation())
        .then(source::anchored_operation())
        .then(source::in_operation())
        .then(source::summon_operation())
        .then(source::on_relations())
}

fn block_entity_full_nbt(block_entity: &dyn crate::block_entity::BlockEntity) -> NbtCompound {
    let mut nbt = NbtCompound::new();
    let entity_pos = block_entity.get_block_pos();
    nbt.insert("id", block_entity.get_type().key.to_string());
    nbt.insert("x", entity_pos.x());
    nbt.insert("y", entity_pos.y());
    nbt.insert("z", entity_pos.z());
    block_entity.save_additional(&mut nbt);
    nbt
}

fn loaded_named_block_position(
    context: &CommandContext,
    arguments: &ParsedArguments,
    name: &'static str,
) -> Result<BlockPos, CommandError> {
    let pos = named_block_position(arguments, name)?;
    if !context.world.is_full_chunk_loaded_at(pos) {
        return Err(position_error("argument.pos.unloaded"));
    }
    if !context.world.is_in_valid_bounds(pos) {
        return Err(position_error("argument.pos.outofworld"));
    }
    Ok(pos)
}

fn position_error(key: &'static str) -> CommandError {
    CommandError::failure(TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed(key),
        fallback: None,
        args: None,
    }))
}

fn block_data_invalid_error() -> CommandError {
    CommandError::failure(TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed("commands.data.block.invalid"),
        fallback: None,
        args: None,
    }))
}

fn entities(arguments: &ParsedArguments) -> Result<Vec<SharedEntity>, CommandError> {
    arguments
        .get::<Vec<SharedEntity>>("targets")
        .or_else(|_| arguments.get::<Vec<SharedEntity>>("entities"))
        .map_err(super::invalid_parsed_argument)
}

fn anchor(arguments: &ParsedArguments) -> Result<EntityAnchor, CommandError> {
    arguments
        .get::<EntityAnchor>("anchor")
        .map_err(super::invalid_parsed_argument)
}

fn position(arguments: &ParsedArguments) -> Result<DVec3, CommandError> {
    arguments
        .get::<DVec3>("pos")
        .map_err(super::invalid_parsed_argument)
}

fn block_position(arguments: &ParsedArguments) -> Result<BlockPos, CommandError> {
    named_block_position(arguments, "pos")
}

fn named_block_position(
    arguments: &ParsedArguments,
    name: &'static str,
) -> Result<BlockPos, CommandError> {
    arguments
        .get::<BlockPos>(name)
        .map_err(super::invalid_parsed_argument)
}

fn biome_value(arguments: &ParsedArguments) -> Result<BiomeArgumentValue, CommandError> {
    arguments
        .get::<BiomeArgumentValue>("biome")
        .map_err(super::invalid_parsed_argument)
}

fn block_predicate(arguments: &ParsedArguments) -> Result<BlockPredicateArgumentValue, CommandError> {
    arguments
        .get::<BlockPredicateArgumentValue>("block")
        .map_err(super::invalid_parsed_argument)
}

fn nbt_path(arguments: &ParsedArguments) -> Result<NbtPath, CommandError> {
    arguments
        .get::<NbtPath>("path")
        .map_err(super::invalid_parsed_argument)
}

fn double(arguments: &ParsedArguments, name: &str) -> Result<f64, CommandError> {
    arguments
        .get::<f64>(name)
        .map_err(super::invalid_parsed_argument)
}

fn scoreboard_objective(
    context: &CommandContext,
    arguments: &ParsedArguments,
    name: &str,
) -> Result<ScoreboardObjective, CommandError> {
    let objective_name = arguments
        .get::<ScoreboardObjectiveName>(name)
        .map_err(super::invalid_parsed_argument)?;
    context
        .server
        .scoreboard
        .objective(objective_name.as_str())
        .ok_or_else(|| {
            CommandError::failure(
                translations::ARGUMENTS_OBJECTIVE_NOT_FOUND
                    .message([TextComponent::from(objective_name.as_str().to_owned())]),
            )
        })
}

fn score_holders(
    arguments: &ParsedArguments,
    name: &str,
) -> Result<ScoreHolderArgumentValue, CommandError> {
    arguments
        .get::<ScoreHolderArgumentValue>(name)
        .map_err(super::invalid_parsed_argument)
}

fn single_score_holder(
    arguments: &ParsedArguments,
    name: &str,
) -> Result<ScoreHolder, CommandError> {
    let holders = score_holders(arguments, name)?;
    let Some(holders) = holders.holders() else {
        return Err(no_score_holders());
    };
    let [holder] = holders else {
        return Err(no_score_holders());
    };
    Ok(holder.to_owned())
}

fn score_holders_or_tracked(
    context: &CommandContext,
    arguments: &ParsedArguments,
    name: &str,
) -> Result<Vec<ScoreHolder>, CommandError> {
    match score_holders(arguments, name)? {
        ScoreHolderArgumentValue::Holders(holders) if holders.is_empty() => {
            Err(no_score_holders())
        }
        ScoreHolderArgumentValue::Holders(holders) => Ok(holders),
        ScoreHolderArgumentValue::Wildcard => {
            let holders = context.server.scoreboard.tracked_holders();
            if holders.is_empty() {
                Err(no_score_holders())
            } else {
                Ok(holders)
            }
        }
    }
}

fn no_score_holders() -> CommandError {
    CommandError::failure(TextComponent::from(
        &translations::ARGUMENT_ENTITY_NOTFOUND_ENTITY,
    ))
}

fn int_range(arguments: &ParsedArguments) -> Result<IntRangeArgumentValue, CommandError> {
    arguments
        .get::<IntRangeArgumentValue>("range")
        .map_err(super::invalid_parsed_argument)
}

fn source_entity(arguments: &ParsedArguments) -> Result<SharedEntity, CommandError> {
    single_entity(arguments, "source")
}

fn single_entity(arguments: &ParsedArguments, name: &'static str) -> Result<SharedEntity, CommandError> {
    let mut entities = arguments
        .get::<Vec<SharedEntity>>(name)
        .map_err(super::invalid_parsed_argument)?;
    if entities.len() != 1 {
        return Err(super::invalid_parsed_argument(
            crate::command::graph::ParsedArgumentError::WrongType {
                name: name.to_owned(),
                expected: "single_entity",
                actual: "entities",
            },
        ));
    }
    Ok(entities.remove(0))
}

fn heightmap(arguments: &ParsedArguments) -> Result<HeightmapType, CommandError> {
    arguments
        .get::<HeightmapType>("heightmap")
        .map_err(super::invalid_parsed_argument)
}

fn rotation(arguments: &ParsedArguments) -> Result<(f32, f32), CommandError> {
    arguments
        .get::<(f32, f32)>("rot")
        .map_err(super::invalid_parsed_argument)
}

fn world(arguments: &ParsedArguments) -> Result<Arc<World>, CommandError> {
    arguments
        .get::<Arc<World>>("dimension")
        .map_err(super::invalid_parsed_argument)
}

fn entity_type(arguments: &ParsedArguments) -> Result<EntityTypeRef, CommandError> {
    arguments
        .get::<EntityTypeRef>("entity")
        .map_err(super::invalid_parsed_argument)
}

fn storage_id(arguments: &ParsedArguments, name: &str) -> Result<Identifier, CommandError> {
    arguments
        .get::<Identifier>(name)
        .map_err(super::invalid_parsed_argument)
}

fn same_world(left: &Arc<World>, right: &Arc<World>) -> bool {
    left.key == right.key
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Weak};

    use glam::DVec3;
    use simdnbt::owned::NbtTag;
    use steel_registry::{entity_type::EntityTypeRef, test_support::init_test_registry, vanilla_entities};
    use steel_utils::{Identifier, nbt::parse_nbt_path};

    use crate::command::graph::CommandGraph;
    use crate::command::requirement::{
        CommandInputContext, CommandSourceKind, PermissionExpr, RequirementContext,
    };
    use crate::command::storage::CommandStorage;
    use crate::entity::{Entity, EntityBase};
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
}
