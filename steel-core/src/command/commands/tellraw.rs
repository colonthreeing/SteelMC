//! Handler for the "tellraw" command.
use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandNodeBuilder, CommandResult, ParsedArgumentError, ParsedArguments, argument, literal,
};
use crate::command::parsers::{ComponentParser, PlayerParser};
use crate::command::sender::CommandSender;
use crate::player::Player;
use std::sync::Arc;
use text_components::TextComponent;

/// Handler for the "tellraw" command.
#[must_use]
pub fn command() -> CommandNodeBuilder {
    literal("tellraw").then(
        argument("targets", PlayerParser::multiple())
            .then(argument("message", ComponentParser).executes(send_tellraw)),
    )
}

fn send_tellraw(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = arguments
        .get::<Vec<Arc<Player>>>("targets")
        .map_err(invalid_parsed_argument)?;
    let message = arguments
        .get::<TextComponent>("message")
        .map_err(invalid_parsed_argument)?;

    let sender = match &context.sender {
        CommandSender::Player(player) => &player.gameprofile.name,
        CommandSender::Console => "Console",
        CommandSender::Rcon => "Rcon",
    };
    log::info!("{}'s tellraw: {:p}", sender, &message);
    for player in targets {
        player.send_message(&message);
    }

    Ok(CommandResult::success())
}

fn invalid_parsed_argument(error: ParsedArgumentError) -> CommandError {
    CommandError::InvalidConsumption(Some(format!("{error:?}")))
}
