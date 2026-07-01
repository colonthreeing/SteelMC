//! Handler for the "tellraw" command.
use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandNodeBuilder, CommandResult, ParsedArguments, argument, literal,
};
use crate::command::parsers::{ComponentParser, PlayerParser, resolve_required_player_targets};
use crate::command::sender::CommandSender;
use crate::command::CommandRegistrationSpec;
use text_components::TextComponent;

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Handler for the "tellraw" command.
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
    literal("tellraw").then(
        argument("targets", PlayerParser::multiple())
            .then(argument("message", ComponentParser).executes(send_tellraw)),
    )
}

fn send_tellraw(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = resolve_required_player_targets(arguments, "targets", context)?;
    let message = arguments
        .get::<TextComponent>("message")
        .map_err(super::invalid_parsed_argument)?;

    let sender = sender_log_name(&context.sender);
    log::info!("{}'s tellraw: {:p}", sender, &message);
    let target_count = targets.len();
    for player in targets {
        player.send_message(&message);
    }

    Ok(CommandResult::from_usize_success_count(target_count))
}

fn sender_log_name(sender: &CommandSender) -> &str {
    match sender {
        CommandSender::Player(player) => &player.gameprofile.name,
        CommandSender::Console => "Console",
        CommandSender::Rcon => "Rcon",
        CommandSender::SuppressedOutput(sender) => sender_log_name(sender),
    }
}
