//! Handler for the "teleport" command.
use std::sync::Arc;

use glam::DVec3;
use steel_utils::{BlockPos, translations};
use text_components::TextComponent;

use crate::command::CommandRegistrationSpec;
use crate::{
    command::{
        context::CommandContext,
        error::CommandError,
        graph::{CommandNodeBuilder, CommandResult, ParsedArguments, argument, literal},
        parsers::{PlayerParser, RotationParser, Vec3Parser, resolve_required_player_targets},
    },
    entity::Entity,
    player::Player,
    world::World,
};

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft()
    .permission_base("teleport")
    .aliases(&["teleport"]);

/// Handler for the "teleport" command.
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
    literal("tp")
        .then(
            argument("targets", PlayerParser::multiple())
                .then(
                    argument("position", Vec3Parser)
                        .executes(teleport_targets_to_position)
                        .then(
                            argument("rotation", RotationParser)
                                .executes(teleport_targets_to_position_with_rotation),
                        ),
                )
                .then(
                    argument("destination", PlayerParser::one())
                        .executes(teleport_targets_to_player),
                ),
        )
        .then(
            argument("location", Vec3Parser)
                .executes(teleport_sender_to_location)
                .then(
                    argument("rotation", RotationParser)
                        .executes(teleport_sender_to_location_with_rotation),
                ),
        )
}

fn teleport_targets_to_position(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments, context)?;
    let pos = position(arguments)?;
    let player = context
        .sender
        .get_player()
        .ok_or(CommandError::InvalidRequirement)?;

    teleport_to_pos(&targets, pos, player.rotation(), context)
}

fn teleport_targets_to_position_with_rotation(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    teleport_to_pos(
        &targets(arguments, context)?,
        position(arguments)?,
        rotation(arguments)?,
        context,
    )
}

fn teleport_targets_to_player(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    teleport_to_player(
        &targets(arguments, context)?,
        &destination(arguments, context)?,
        context,
    )
}

fn teleport_sender_to_location(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let player = context
        .player
        .clone()
        .ok_or(CommandError::InvalidRequirement)?;
    let rotation = player.rotation();

    teleport_to_pos(&[player], position(arguments)?, rotation, context)
}

fn teleport_sender_to_location_with_rotation(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let player = context
        .player
        .clone()
        .ok_or(CommandError::InvalidRequirement)?;

    teleport_to_pos(
        &[player],
        position(arguments)?,
        rotation(arguments)?,
        context,
    )
}

fn targets(
    arguments: &ParsedArguments,
    context: &CommandContext,
) -> Result<Vec<Arc<Player>>, CommandError> {
    resolve_required_player_targets(arguments, "targets", context)
}

fn destination(
    arguments: &ParsedArguments,
    context: &CommandContext,
) -> Result<Vec<Arc<Player>>, CommandError> {
    resolve_required_player_targets(arguments, "destination", context)
}

fn position(arguments: &ParsedArguments) -> Result<DVec3, CommandError> {
    arguments
        .get::<DVec3>("position")
        .or_else(|_| arguments.get::<DVec3>("location"))
        .map_err(super::invalid_parsed_argument)
}

fn rotation(arguments: &ParsedArguments) -> Result<(f32, f32), CommandError> {
    arguments
        .get::<(f32, f32)>("rotation")
        .map_err(super::invalid_parsed_argument)
}

fn teleport_to_pos(
    targets: &[Arc<Player>],
    pos: DVec3,
    rotation: (f32, f32),
    ctx: &mut CommandContext,
) -> Result<CommandResult, CommandError> {
    if !World::is_in_spawnable_bounds(BlockPos::from(pos)) {
        return Err(CommandError::failure(
            translations::COMMANDS_TELEPORT_INVALID_POSITION.message([] as [TextComponent; 0]),
        ));
    }

    let targets = current_players(targets, ctx)?;
    for player in &targets {
        teleport_player(player, pos, rotation.0, rotation.1)?;
    }

    if let [target] = targets.as_slice() {
        ctx.sender.send_message(
            &translations::COMMANDS_TELEPORT_SUCCESS_LOCATION_SINGLE
                .message([
                    TextComponent::from(target.gameprofile.name.clone()),
                    TextComponent::from(format!("{:.2}", pos.x)),
                    TextComponent::from(format!("{:.2}", pos.y)),
                    TextComponent::from(format!("{:.2}", pos.z)),
                ])
                .into(),
        );
    } else {
        ctx.sender.send_message(
            &translations::COMMANDS_TELEPORT_SUCCESS_LOCATION_MULTIPLE
                .message([
                    TextComponent::from(format!("{}", targets.len())),
                    TextComponent::from(format!("{:.2}", pos.x)),
                    TextComponent::from(format!("{:.2}", pos.y)),
                    TextComponent::from(format!("{:.2}", pos.z)),
                ])
                .into(),
        );
    }
    Ok(CommandResult::from_usize_success_count(targets.len()))
}

fn teleport_to_player(
    targets: &[Arc<Player>],
    destination: &[Arc<Player>],
    ctx: &mut CommandContext,
) -> Result<CommandResult, CommandError> {
    let Some(destination) = destination.first() else {
        return Err(no_player_found());
    };
    let destination = current_player(destination, ctx).ok_or_else(no_player_found)?;

    let pos = destination.position();
    let (yaw, pitch) = destination.rotation();

    let targets = current_players(targets, ctx)?;
    for player in &targets {
        teleport_player(player, pos, yaw, pitch)?;
    }

    if let [target] = targets.as_slice() {
        ctx.sender.send_message(
            &translations::COMMANDS_TELEPORT_SUCCESS_ENTITY_SINGLE
                .message([
                    TextComponent::from(target.gameprofile.name.clone()),
                    TextComponent::from(destination.gameprofile.name.clone()),
                ])
                .into(),
        );
    } else {
        ctx.sender.send_message(
            &translations::COMMANDS_TELEPORT_SUCCESS_ENTITY_MULTIPLE
                .message([
                    TextComponent::from(format!("{}", targets.len())),
                    TextComponent::from(destination.gameprofile.name.clone()),
                ])
                .into(),
        );
    }
    Ok(CommandResult::from_usize_success_count(targets.len()))
}

fn current_players(
    players: &[Arc<Player>],
    ctx: &CommandContext,
) -> Result<Vec<Arc<Player>>, CommandError> {
    let current_players = ctx.server.get_players();
    let players = players
        .iter()
        .filter_map(|player| {
            current_players
                .iter()
                .find(|current| current.uuid() == player.uuid())
                .cloned()
        })
        .collect::<Vec<_>>();
    if players.is_empty() {
        return Err(no_player_found());
    }
    Ok(players)
}

fn current_player(player: &Player, ctx: &CommandContext) -> Option<Arc<Player>> {
    ctx.server
        .get_players()
        .into_iter()
        .find(|current| current.uuid() == player.uuid())
}

fn no_player_found() -> CommandError {
    CommandError::failure("No player was found")
}

fn teleport_player(player: &Player, pos: DVec3, yaw: f32, pitch: f32) -> Result<(), CommandError> {
    player.teleport(pos, yaw, pitch).map_err(|error| {
        CommandError::failure(format!(
            "Failed to teleport {}: {error}",
            player.gameprofile.name
        ))
    })?;
    player.reset_flying_ticks();

    if !player.is_fall_flying() {
        let velocity = player.velocity();
        player.set_velocity(DVec3::new(velocity.x, 0.0, velocity.z));
        player.set_on_ground(true);
    }

    Ok(())
}
