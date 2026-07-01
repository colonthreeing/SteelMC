//! Handler for the "function" command.

use std::borrow::Cow;

use simdnbt::owned::NbtCompound;
use text_components::{TextComponent, translation::TranslatedMessage};

use crate::command::{
    CommandRegistrationSpec,
    context::CommandContext,
    error::CommandError,
    functions::CommandFunction,
    graph::{
        CommandExecutionStep, CommandFunctionArgumentValue, CommandNodeBuilder, ParsedArgumentError,
        ParsedArguments, argument, literal,
    },
    parsers::{CommandFunctionParser, NbtCompoundParser},
};

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Handler for the "function" command.
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
    literal("function").then(
        argument("name", CommandFunctionParser)
            .executes_step_with_budget(execute_function)
            .then(
                argument("arguments", NbtCompoundParser)
                    .executes_step_with_budget(execute_function),
            ),
    )
}

fn execute_function(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    _budget: &mut crate::command::CommandExecutionBudget,
) -> Result<CommandExecutionStep, CommandError> {
    let target = arguments
        .get::<CommandFunctionArgumentValue>("name")
        .map_err(super::invalid_parsed_argument)?;
    let macro_arguments = function_macro_arguments(arguments)?;
    let functions = {
        let registry = context.server.command_functions.read();
        registry
            .resolve_argument(&target)
            .map_err(|error| CommandError::failure(error.to_string()))?
    };
    if functions.is_empty() {
        return Err(no_functions_error(&target));
    }

    send_scheduled_message(context, &functions);
    Ok(CommandExecutionStep::CallFunctions {
        functions,
        arguments: macro_arguments,
    })
}

fn function_macro_arguments(arguments: &ParsedArguments) -> Result<Option<NbtCompound>, CommandError> {
    match arguments.get::<NbtCompound>("arguments") {
        Ok(arguments) => Ok(Some(arguments)),
        Err(ParsedArgumentError::Missing(_)) => Ok(None),
        Err(error) => Err(super::invalid_parsed_argument(error)),
    }
}

fn no_functions_error(target: &CommandFunctionArgumentValue) -> CommandError {
    CommandError::failure(TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed("commands.function.scheduled.no_functions"),
        fallback: None,
        args: Some(Box::new([TextComponent::from(function_argument_text(
            target,
        ))])),
    }))
}

fn send_scheduled_message(context: &CommandContext, functions: &[CommandFunction]) {
    if context.is_output_suppressed() {
        return;
    }

    let message = if functions.len() == 1 {
        translated(
            "commands.function.scheduled.single",
            [TextComponent::from(functions[0].id().to_string())],
        )
    } else {
        translated(
            "commands.function.scheduled.multiple",
            [TextComponent::from(function_list_text(functions))],
        )
    };
    context.sender.send_message(&message);
}

fn translated<const N: usize>(key: &'static str, args: [TextComponent; N]) -> TextComponent {
    TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed(key),
        fallback: None,
        args: Some(Box::new(args)),
    })
}

fn function_argument_text(target: &CommandFunctionArgumentValue) -> String {
    match target {
        CommandFunctionArgumentValue::Function(id) => id.to_string(),
        CommandFunctionArgumentValue::Tag(id) => format!("#{id}"),
    }
}

fn function_list_text(functions: &[CommandFunction]) -> String {
    functions
        .iter()
        .map(|function| function.id().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}
