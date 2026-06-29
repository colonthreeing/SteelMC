//! Handler for the `setworldspawn` command.

use std::borrow::Cow;

use text_components::TextComponent;
use text_components::translation::TranslatedMessage;

use crate::command::CommandRegistrationSpec;
use crate::command::{
    context::CommandContext,
    error::CommandError,
    graph::{CommandNodeBuilder, CommandResult, ParsedArguments, argument, literal},
    parsers::{BlockPosParser, RotationParser},
};
use crate::level_data::RespawnData;
use crate::world::World;
use steel_utils::BlockPos;

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Handler for the `setworldspawn` command.
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
    literal("setworldspawn").executes(set_default_spawn).then(
        argument("pos", BlockPosParser)
            .executes(set_spawn_at_pos)
            .then(argument("rotation", RotationParser).executes(set_spawn_at_pos_rotation)),
    )
}

fn set_default_spawn(
    context: &mut CommandContext,
    _: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    set_spawn(context, BlockPos::from(context.position), (0.0, 0.0))
}

fn set_spawn_at_pos(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let pos = arguments
        .get::<BlockPos>("pos")
        .map_err(super::invalid_parsed_argument)?;

    set_spawn(context, pos, (0.0, 0.0))
}

fn set_spawn_at_pos_rotation(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let pos = arguments
        .get::<BlockPos>("pos")
        .map_err(super::invalid_parsed_argument)?;
    let rotation = arguments
        .get::<(f32, f32)>("rotation")
        .map_err(super::invalid_parsed_argument)?;

    set_spawn(context, pos, rotation)
}

fn set_spawn(
    context: &mut CommandContext,
    pos: BlockPos,
    rotation: (f32, f32),
) -> Result<CommandResult, CommandError> {
    if !World::is_in_spawnable_bounds(pos) {
        return Err(CommandError::failure(translated(
            "argument.pos.outofbounds",
            [],
        )));
    }

    let respawn_data = RespawnData::of(context.world.key.clone(), pos, rotation.0, rotation.1);
    context
        .server
        .set_respawn_data(respawn_data.clone())
        .map_err(command_failed)?;

    context.sender.send_message(&translated(
        "commands.setworldspawn.success",
        [
            TextComponent::from(pos.x().to_string()),
            TextComponent::from(pos.y().to_string()),
            TextComponent::from(pos.z().to_string()),
            TextComponent::from(respawn_data.yaw.to_string()),
            TextComponent::from(respawn_data.pitch.to_string()),
            TextComponent::from(context.world.key.to_string()),
        ],
    ));

    Ok(CommandResult::success())
}

fn command_failed(error: String) -> CommandError {
    CommandError::failure(error)
}

fn translated<const N: usize>(key: &'static str, args: [TextComponent; N]) -> TextComponent {
    TranslatedMessage {
        key: Cow::Borrowed(key),
        fallback: None,
        args: Some(Box::new(args)),
    }
    .component()
}
