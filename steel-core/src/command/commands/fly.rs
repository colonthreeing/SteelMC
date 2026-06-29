//! Handler for the "flyspeed" command.
use std::slice;
use std::sync::Arc;

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    BoolParser, CommandNodeBuilder, CommandResult, FloatParser, ParsedArguments, argument, literal,
};
use crate::command::parsers::PlayerParser;
use crate::command::sender::CommandSender;
use crate::command::CommandRegistrationSpec;
use crate::player::Player;
use text_components::TextComponent;

const MAX_FLY_SPEED: f32 = 30f32;

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::steel();

/// Handler for the "flyspeed" command.
#[must_use]
pub fn command() -> CommandNodeBuilder {
    literal("fly")
        .executes(toggle_sender_fly)
        .then(
            argument("target", PlayerParser::multiple())
                .executes(toggle_target_fly)
                .then(argument("value", BoolParser).executes(set_target_fly))
                .then(
                    literal("speed").executes(query_target_flying_speed).then(
                        argument(
                            "speed",
                            FloatParser::bounded(Some(0.0), Some(MAX_FLY_SPEED)),
                        )
                        .executes(set_target_flying_speed),
                    ),
                ),
        )
        .then(
            literal("speed").executes(query_sender_flying_speed).then(
                argument(
                    "speed",
                    FloatParser::bounded(Some(0.0), Some(MAX_FLY_SPEED)),
                )
                .executes(set_sender_flying_speed),
            ),
        )
}

fn toggle_sender_fly(
    context: &mut CommandContext,
    _: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let player = context
        .sender
        .get_player()
        .ok_or(CommandError::InvalidRequirement)?;

    toggle_fly(slice::from_ref(player));

    Ok(CommandResult::success())
}

fn toggle_target_fly(
    _context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    toggle_fly(&targets);

    Ok(CommandResult::success())
}

fn set_target_fly(
    _context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let value = arguments
        .get::<bool>("value")
        .map_err(super::invalid_parsed_argument)?;
    set_fly(&targets, value);

    Ok(CommandResult::success())
}

fn query_target_flying_speed(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    query_flying_speed(&targets, &context.sender);

    Ok(CommandResult::success())
}

fn set_target_flying_speed(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let speed = speed(arguments)?;
    set_flying_speed(&targets, speed, &context.sender);

    Ok(CommandResult::success())
}

fn query_sender_flying_speed(
    context: &mut CommandContext,
    _: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let player = context
        .sender
        .get_player()
        .ok_or(CommandError::InvalidRequirement)?;

    query_flying_speed(slice::from_ref(player), &context.sender);

    Ok(CommandResult::success())
}

fn set_sender_flying_speed(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let player = context
        .sender
        .get_player()
        .ok_or(CommandError::InvalidRequirement)?;
    let speed = speed(arguments)?;

    set_flying_speed(slice::from_ref(player), speed, &context.sender);

    Ok(CommandResult::success())
}

fn targets(arguments: &ParsedArguments) -> Result<Vec<Arc<Player>>, CommandError> {
    arguments
        .get::<Vec<Arc<Player>>>("target")
        .map_err(super::invalid_parsed_argument)
}

fn speed(arguments: &ParsedArguments) -> Result<f32, CommandError> {
    arguments
        .get::<f32>("speed")
        .map_err(super::invalid_parsed_argument)
}

fn toggle_fly(targets: &[Arc<Player>]) {
    for target in targets {
        {
            let mut lock = target.abilities.lock();
            lock.may_fly = !lock.may_fly;
            if !lock.may_fly {
                lock.flying = false;
            }
        }
        target.send_abilities();
    }
}

fn set_fly(targets: &[Arc<Player>], value: bool) {
    for target in targets {
        {
            let mut lock = target.abilities.lock();
            lock.may_fly = value;
            if !value {
                lock.flying = false;
            }
        }
        target.send_abilities();
    }
}

fn set_flying_speed(targets: &[Arc<Player>], multiplier: f32, sender: &CommandSender) {
    let speed = multiplier * 0.05;
    for target in targets {
        target.set_flying_speed(speed);
        target.send_abilities();
        sender.send_message(&TextComponent::from(format!(
            "Set flying speed for player '{}' to {multiplier:.1}x ({speed:.3})",
            target.gameprofile.name.clone()
        )));
    }
}

fn query_flying_speed(targets: &[Arc<Player>], sender: &CommandSender) {
    for target in targets {
        let speed = target.get_flying_speed();
        let multiplier = speed / 0.05; // Show as multiplier of default speed

        sender.send_message(&TextComponent::from(format!(
            "Current flying speed for player '{}': {multiplier:.1}x ({speed:.3})",
            target.gameprofile.name.clone()
        )));
    }
}
