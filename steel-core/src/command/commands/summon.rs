//! Handler for the "summon" command.

use std::borrow::Cow;
use std::sync::Arc;

use glam::DVec3;
use steel_registry::entity_type::EntityTypeRef;
use steel_utils::types::Difficulty;
use steel_utils::{BlockPos, translations};
use text_components::TextComponent;
use text_components::translation::TranslatedMessage;

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandNodeBuilder, CommandResult, ParsedArguments, argument, literal,
};
use crate::command::parsers::{EntitySummonParser, Vec3Parser};
use crate::command::CommandRegistrationSpec;
use crate::entity::{
    AddEntityError, ENTITIES, Entity, EntitySpawnReason, SharedEntity, next_entity_id,
};
use crate::world::World;

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Handler for the "summon" command.
#[must_use]
pub fn command() -> CommandNodeBuilder {
    literal("summon").then(
        argument("entity", EntitySummonParser)
            .executes(summon_at_source)
            .then(argument("pos", Vec3Parser).executes(summon_at_pos)),
    )
}

fn summon_at_source(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    summon_entity(context, entity_type(arguments)?, context.position)
}

fn summon_at_pos(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    summon_entity(context, entity_type(arguments)?, position(arguments)?)
}

fn entity_type(arguments: &ParsedArguments) -> Result<EntityTypeRef, CommandError> {
    arguments
        .get::<EntityTypeRef>("entity")
        .map_err(super::invalid_parsed_argument)
}

fn position(arguments: &ParsedArguments) -> Result<DVec3, CommandError> {
    arguments
        .get::<DVec3>("pos")
        .map_err(super::invalid_parsed_argument)
}

fn summon_entity(
    context: &mut CommandContext,
    entity_type: EntityTypeRef,
    pos: DVec3,
) -> Result<CommandResult, CommandError> {
    let entity = create_entity(context, entity_type, pos)?;
    context.sender.send_message(
        &translations::COMMANDS_SUMMON_SUCCESS
            .message([entity_display_name(entity.as_ref())])
            .into(),
    );
    Ok(CommandResult::success())
}

fn create_entity(
    context: &CommandContext,
    entity_type: EntityTypeRef,
    pos: DVec3,
) -> Result<SharedEntity, CommandError> {
    let block_pos = BlockPos::containing(pos.x, pos.y, pos.z);
    if !World::is_in_spawnable_bounds(block_pos) {
        return Err(command_failed(
            translations::COMMANDS_SUMMON_INVALID_POSITION.msg(),
        ));
    }

    if context.world.difficulty() == Difficulty::Peaceful && !entity_type.allowed_in_peaceful {
        return Err(command_failed(
            translations::COMMANDS_SUMMON_FAILED_PEACEFUL.msg(),
        ));
    }

    let world = Arc::clone(&context.world);
    let Some(entity) = ENTITIES.create(entity_type, next_entity_id(), pos, Arc::downgrade(&world))
    else {
        return Err(command_failed(translations::COMMANDS_SUMMON_FAILED.msg()));
    };

    if let Some(mob) = entity.as_mob() {
        let _ = mob.finalize_spawn(&world, EntitySpawnReason::Command, None);
    }

    match world.try_add_entity(Arc::clone(&entity)) {
        Ok(()) => Ok(entity),
        Err(AddEntityError::DuplicateUuid { .. }) => Err(command_failed(
            translations::COMMANDS_SUMMON_FAILED_UUID.msg(),
        )),
        Err(_) => Err(command_failed(translations::COMMANDS_SUMMON_FAILED.msg())),
    }
}

fn command_failed(message: TranslatedMessage) -> CommandError {
    CommandError::failure(message)
}

fn entity_display_name(entity: &dyn Entity) -> TextComponent {
    entity
        .custom_name()
        .unwrap_or_else(|| entity_type_display_name(entity.entity_type()))
}

fn entity_type_display_name(entity_type: EntityTypeRef) -> TextComponent {
    TextComponent::translated(TranslatedMessage {
        key: Cow::Owned(format!(
            "entity.{}.{}",
            entity_type.key.namespace, entity_type.key.path
        )),
        fallback: None,
        args: None,
    })
}
