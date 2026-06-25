//! Handler for the "domain" command.

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandNodeBuilder, CommandResult, ParsedArgumentError, ParsedArguments, argument, literal,
};
use crate::command::parsers::DomainParser;
use text_components::TextComponent;

/// Handler for switching to another configured domain.
#[must_use]
pub fn command() -> CommandNodeBuilder {
    literal("domain").then(argument("domain", DomainParser).executes(switch_domain))
}

fn switch_domain(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let domain = arguments
        .get::<String>("domain")
        .map_err(invalid_parsed_argument)?;
    let player = context
        .sender
        .get_player()
        .cloned()
        .ok_or(CommandError::InvalidRequirement)?;
    let server = context.server.clone();
    server
        .queue_domain_switch(player, domain.clone())
        .map_err(|error| CommandError::CommandFailed(Box::new(TextComponent::plain(error))))?;

    context.sender.send_message(&TextComponent::plain(format!(
        "Switching to domain {domain}"
    )));
    Ok(CommandResult::success())
}

fn invalid_parsed_argument(error: ParsedArgumentError) -> CommandError {
    CommandError::InvalidConsumption(Some(format!("{error:?}")))
}
