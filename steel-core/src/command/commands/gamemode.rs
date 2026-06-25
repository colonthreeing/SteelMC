//! Handler for the "gamemode" command.
use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandNodeBuilder, CommandResult, ParsedArgumentError, ParsedArguments, argument, literal,
};
use crate::command::parsers::{GameModeParser, PlayerParser};
use crate::command::{CommandRegistration, CommandRegistrationError};
use crate::entity::Entity;
use crate::player::Player;
use std::sync::Arc;
use steel_utils::translations;
use steel_utils::types::GameType;
use text_components::TextComponent;
use text_components::translation::Translation;

pub(crate) fn registration() -> Result<CommandRegistration, CommandRegistrationError> {
    CommandRegistration::minecraft(command())
}

/// Handler for the "gamemode" command.
#[must_use]
pub fn command() -> CommandNodeBuilder {
    literal("gamemode").then(
        argument("gamemode", GameModeParser)
            .requires_argument_permission::<GameType>("gamemode")
            .executes(set_own_game_mode)
            .then(argument("targets", PlayerParser::multiple()).executes(set_target_game_mode)),
    )
}

fn set_own_game_mode(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let gamemode = arguments
        .get::<GameType>("gamemode")
        .map_err(invalid_parsed_argument)?;

    let player = context
        .sender
        .get_player()
        .ok_or(CommandError::InvalidRequirement)?;

    player.set_game_mode(gamemode);

    Ok(CommandResult::success())
}

fn set_target_game_mode(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let gamemode = arguments
        .get::<GameType>("gamemode")
        .map_err(invalid_parsed_argument)?;
    let targets = arguments
        .get::<Vec<Arc<Player>>>("targets")
        .map_err(invalid_parsed_argument)?;

    let mode_translation = get_gamemode_translation(gamemode);

    for target in targets {
        if target.set_game_mode(gamemode) {
            let sender_is_target = if let Some(sender_player) = context.sender.get_player() {
                sender_player.id() == target.id()
            } else {
                false
            };

            if !sender_is_target {
                context.sender.send_message(
                    &translations::COMMANDS_GAMEMODE_SUCCESS_OTHER
                        .message([
                            TextComponent::plain(target.gameprofile.name.clone()),
                            TextComponent::from(mode_translation),
                        ])
                        .into(),
                );
            }
        }
    }

    Ok(CommandResult::success())
}

fn invalid_parsed_argument(error: ParsedArgumentError) -> CommandError {
    CommandError::InvalidConsumption(Some(format!("{error:?}")))
}

/// Retrieves the translation for a `GameType`
#[must_use]
pub fn get_gamemode_translation(gamemode: GameType) -> &'static Translation<0> {
    match gamemode {
        GameType::Survival => &translations::GAME_MODE_SURVIVAL,
        GameType::Creative => &translations::GAME_MODE_CREATIVE,
        GameType::Adventure => &translations::GAME_MODE_ADVENTURE,
        GameType::Spectator => &translations::GAME_MODE_SPECTATOR,
    }
}
