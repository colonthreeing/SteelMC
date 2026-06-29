//! Handler for the "execute" command.
//!
//! TODO: This is a partial implementation. Missing subcommands include:
//! - `as` (execute as another entity)
//! - `at` (execute at another entity's position)
//! - `positioned` (execute at specific coordinates)
//! - `if`/`unless` (conditional execution)
//! - `store` (store command results)
//! - `facing` (face towards entity or coordinates)
//! - `align` (align position to block grid)
//! - `in` (execute in another world; vanilla command docs call these dimensions)
//! - `summon` (execute as newly summoned entity)
//! - `on` (execute on related entities)

use crate::command::context::{CommandContext, EntityAnchor};
use crate::command::error::CommandError;
use crate::command::graph::{
    AnchorParser, CommandNodeBuilder, CommandRedirectTarget, CommandResult, ParsedArguments,
    argument, literal,
};
use crate::command::parsers::RotationParser;
use crate::command::CommandRegistrationSpec;

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Handler for the "execute" command.
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
    literal("execute")
        .then(literal("anchored").then(
            argument("anchor", AnchorParser).redirects(CommandRedirectTarget::Current, set_anchor),
        ))
        .then(literal("rotated").then(
            argument("rot", RotationParser).redirects(CommandRedirectTarget::Current, set_rotation),
        ))
        .then(
            literal("run").redirects(CommandRedirectTarget::All, |_, _| {
                Ok(CommandResult::success())
            }),
        )
}

fn set_anchor(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    context.anchor = arguments
        .get::<EntityAnchor>("anchor")
        .map_err(super::invalid_parsed_argument)?;

    Ok(CommandResult::success())
}

fn set_rotation(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    context.rotation = Some(
        arguments
            .get::<(f32, f32)>("rot")
            .map_err(super::invalid_parsed_argument)?,
    );

    Ok(CommandResult::success())
}
