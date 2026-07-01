//! Handler for the "return" command.

use crate::command::{
    CommandRegistrationSpec,
    context::CommandContext,
    error::CommandError,
    graph::{
        CommandNodeBuilder, CommandRedirectTarget, CommandResult, IntegerParser, ParsedArguments,
        argument, literal,
    },
};

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Handler for the "return" command.
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
    literal("return")
        .then(argument("value", IntegerParser::new()).executes(return_value))
        .then(literal("fail").executes(return_fail))
        .then(literal("run").redirects_returning(CommandRedirectTarget::All, |_, _| {
            Ok(CommandResult::success())
        }))
}

fn return_value(
    _context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    Ok(CommandResult::return_success(
        arguments
            .get::<i32>("value")
            .map_err(super::invalid_parsed_argument)?,
    ))
}

fn return_fail(
    _context: &mut CommandContext,
    _arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    Ok(CommandResult::return_failure())
}
