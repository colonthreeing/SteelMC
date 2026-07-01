//! Handler for the "execute" command.
//!
//! Function conditions currently run non-macro command functions. Function
//! macros remain rejected by the command-function loader until macro
//! instantiation has a runtime.
//! Data component predicates without concrete Steel component storage are also
//! omitted.

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
    CommandResult, DoubleRangeArgumentValue, IntRangeArgumentValue, ItemPredicateArgumentValue,
    ItemSlotRangeArgumentValue, LootPredicateArgumentValue, ParsedArguments,
    ScoreHolderArgumentValue, ScoreboardObjectiveName, literal,
};
use crate::command::parsers::{
    ScoreHolderWildcardExpansion, WorldArgumentValue, resolve_optional_entity_targets,
    resolve_required_entity_targets, resolve_score_holders,
};
use crate::command::requirement::CommandInputContext;
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

fn optional_entities(
    context: &dyn CommandInputContext,
    arguments: &ParsedArguments,
) -> Result<Vec<SharedEntity>, CommandError> {
    resolve_optional_entity_targets(arguments, "targets", context)
        .or_else(|_| resolve_optional_entity_targets(arguments, "entities", context))
}

fn required_entities(
    context: &dyn CommandInputContext,
    arguments: &ParsedArguments,
) -> Result<Vec<SharedEntity>, CommandError> {
    resolve_required_entity_targets(arguments, "targets", context)
        .or_else(|_| resolve_required_entity_targets(arguments, "entities", context))
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

fn item_slots(arguments: &ParsedArguments) -> Result<ItemSlotRangeArgumentValue, CommandError> {
    arguments
        .get::<ItemSlotRangeArgumentValue>("slots")
        .map_err(super::invalid_parsed_argument)
}

fn item_predicate(
    arguments: &ParsedArguments,
) -> Result<ItemPredicateArgumentValue, CommandError> {
    arguments
        .get::<ItemPredicateArgumentValue>("item_predicate")
        .map_err(super::invalid_parsed_argument)
}

fn loot_predicate(arguments: &ParsedArguments) -> Result<LootPredicateArgumentValue, CommandError> {
    arguments
        .get::<LootPredicateArgumentValue>("predicate")
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
    context: &dyn CommandInputContext,
    arguments: &ParsedArguments,
    name: &str,
) -> Result<Vec<ScoreHolder>, CommandError> {
    let value = arguments
        .get::<ScoreHolderArgumentValue>(name)
        .map_err(super::invalid_parsed_argument)?;
    resolve_score_holders(value, context, ScoreHolderWildcardExpansion::Empty)
}

fn single_score_holder(
    context: &dyn CommandInputContext,
    arguments: &ParsedArguments,
    name: &str,
) -> Result<ScoreHolder, CommandError> {
    let mut holders = score_holders(context, arguments, name)?;
    if holders.len() != 1 {
        return Err(no_score_holders());
    }
    Ok(holders.remove(0))
}

fn score_holders_or_tracked(
    context: &CommandContext,
    arguments: &ParsedArguments,
    name: &str,
) -> Result<Vec<ScoreHolder>, CommandError> {
    let value = arguments
        .get::<ScoreHolderArgumentValue>(name)
        .map_err(super::invalid_parsed_argument)?;
    let wildcard = value.is_wildcard();
    let holders = resolve_score_holders(
        value,
        context,
        ScoreHolderWildcardExpansion::TrackedHolders,
    )?;
    if holders.is_empty() {
        if wildcard {
            Err(empty_score_holder_wildcard())
        } else {
            Err(no_score_holders())
        }
    } else {
        Ok(holders)
    }
}

fn empty_score_holder_wildcard() -> CommandError {
    CommandError::failure(TextComponent::from(
        &translations::ARGUMENT_SCORE_HOLDER_EMPTY,
    ))
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

fn double_range(arguments: &ParsedArguments) -> Result<DoubleRangeArgumentValue, CommandError> {
    arguments
        .get::<DoubleRangeArgumentValue>("range")
        .map_err(super::invalid_parsed_argument)
}

fn source_entity(
    context: &dyn CommandInputContext,
    arguments: &ParsedArguments,
) -> Result<SharedEntity, CommandError> {
    single_entity(context, arguments, "source")
}

fn single_entity(
    context: &dyn CommandInputContext,
    arguments: &ParsedArguments,
    name: &'static str,
) -> Result<SharedEntity, CommandError> {
    let mut entities = resolve_required_entity_targets(arguments, name, context)?;
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

fn world(context: &CommandContext, arguments: &ParsedArguments) -> Result<Arc<World>, CommandError> {
    let world = arguments
        .get::<WorldArgumentValue>("dimension")
        .map_err(super::invalid_parsed_argument)?;
    world.resolve(context)
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
#[path = "execute/tests.rs"]
mod tests;
