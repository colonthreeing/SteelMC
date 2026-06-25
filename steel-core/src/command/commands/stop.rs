//! Handler for the "stop" command.
use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{CommandNodeBuilder, CommandResult, ParsedArguments, literal};

/// Handler for the "stop" command.
#[must_use]
pub fn command() -> CommandNodeBuilder {
    literal("stop").executes(stop_server)
}

fn stop_server(
    context: &mut CommandContext,
    _: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    context.server.cancel_token.cancel();
    Ok(CommandResult::success())
}
