//! Handler for the `stopwatch` command.

use std::borrow::Cow;

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use steel_utils::Identifier;
use text_components::{TextComponent, translation::TranslatedMessage};

use crate::{
    command::{
        CommandRegistrationSpec,
        context::CommandContext,
        error::CommandError,
        graph::{
            CommandArgumentClientParser, CommandArgumentParser, CommandNodeBuilder,
            CommandParseError, CommandParseErrorKind, CommandResult, DoubleParser,
            ParsedArgument, ParsedArguments, argument, literal,
        },
        parsers::parse_resource_identifier,
        reader::CommandReader,
        requirement::CommandInputContext,
        suggestions::matches_suggestion_substr,
    },
};

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Handler for the `stopwatch` command.
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
    literal("stopwatch")
        .then(literal("create").then(argument("id", StopwatchIdParser::any()).executes(create)))
        .then(
            literal("query").then(
                argument("id", StopwatchIdParser::existing())
                    .executes(query_default_scale)
                    .then(argument("scale", DoubleParser::new()).executes(query_scaled)),
            ),
        )
        .then(literal("restart").then(argument("id", StopwatchIdParser::existing()).executes(restart)))
        .then(literal("remove").then(argument("id", StopwatchIdParser::existing()).executes(remove)))
}

fn create(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let id = stopwatch_id(arguments)?;
    if !context.server.stopwatches.write().create(id.clone()) {
        return Err(stopwatch_already_exists(&id));
    }

    context.sender.send_message(&translated(
        "commands.stopwatch.create.success",
        [TextComponent::from(id.to_string())],
    ));
    Ok(CommandResult::from_return_value(1))
}

fn query_default_scale(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    query(context, arguments, 1.0)
}

fn query_scaled(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let scale = arguments
        .get::<f64>("scale")
        .map_err(super::invalid_parsed_argument)?;
    query(context, arguments, scale)
}

fn query(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    scale: f64,
) -> Result<CommandResult, CommandError> {
    let id = stopwatch_id(arguments)?;
    let elapsed_seconds = context
        .server
        .stopwatches
        .read()
        .get(&id)
        .ok_or_else(|| stopwatch_does_not_exist(&id))?
        .elapsed_seconds();

    context.sender.send_message(&translated(
        "commands.stopwatch.query",
        [
            TextComponent::from(id.to_string()),
            TextComponent::from(elapsed_seconds.to_string()),
        ],
    ));
    Ok(CommandResult::from_return_value(
        (elapsed_seconds * scale) as i32,
    ))
}

fn restart(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let id = stopwatch_id(arguments)?;
    if !context.server.stopwatches.write().restart(&id) {
        return Err(stopwatch_does_not_exist(&id));
    }

    context.sender.send_message(&translated(
        "commands.stopwatch.restart.success",
        [TextComponent::from(id.to_string())],
    ));
    Ok(CommandResult::from_return_value(1))
}

fn remove(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let id = stopwatch_id(arguments)?;
    if !context.server.stopwatches.write().remove(&id) {
        return Err(stopwatch_does_not_exist(&id));
    }

    context.sender.send_message(&translated(
        "commands.stopwatch.remove.success",
        [TextComponent::from(id.to_string())],
    ));
    Ok(CommandResult::from_return_value(1))
}

fn stopwatch_id(arguments: &ParsedArguments) -> Result<Identifier, CommandError> {
    arguments
        .get::<Identifier>("id")
        .map_err(super::invalid_parsed_argument)
}

pub(in crate::command::commands) fn stopwatch_does_not_exist(id: &Identifier) -> CommandError {
    CommandError::failure(translated(
        "commands.stopwatch.does_not_exist",
        [TextComponent::from(id.to_string())],
    ))
}

fn stopwatch_already_exists(id: &Identifier) -> CommandError {
    CommandError::failure(translated(
        "commands.stopwatch.already_exists",
        [TextComponent::from(id.to_string())],
    ))
}

fn translated<const N: usize>(key: &'static str, args: [TextComponent; N]) -> TextComponent {
    TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed(key),
        fallback: None,
        args: Some(Box::new(args)),
    })
}

/// Stopwatch id parser with server-backed id suggestions.
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::command::commands) struct StopwatchIdParser {
    suggest_existing: bool,
}

impl StopwatchIdParser {
    /// Creates a parser for a new or existing stopwatch id without suggestions.
    #[must_use]
    pub(in crate::command::commands) const fn any() -> Self {
        Self {
            suggest_existing: false,
        }
    }

    /// Creates a parser for an existing stopwatch id with server suggestions.
    #[must_use]
    pub(in crate::command::commands) const fn existing() -> Self {
        Self {
            suggest_existing: true,
        }
    }
}

impl CommandArgumentParser for StopwatchIdParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_token()?;
        let Some(id) = parse_resource_identifier(&raw) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidIdentifier(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::Identifier(id))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(
            ArgumentType::Identifier,
            self.suggest_existing
                .then_some(SuggestionType::AskServer),
        )
    }

    fn parsed_type(&self) -> &'static str {
        "identifier"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        if !self.suggest_existing {
            return Vec::new();
        }

        let Some(server) = context.server() else {
            return Vec::new();
        };

        server
            .stopwatches
            .read()
            .ids()
            .into_iter()
            .map(|id| id.to_string())
            .filter(|id| matches_identifier_suggestion(prefix, id))
            .map(SuggestionEntry::new)
            .collect()
    }

    fn examples(&self) -> &'static [&'static str] {
        &["foo", "minecraft:foo"]
    }
}

fn matches_identifier_suggestion(prefix: &str, value: &str) -> bool {
    let prefix = prefix.strip_prefix("minecraft:").unwrap_or(prefix);
    let value = value.strip_prefix("minecraft:").unwrap_or(value);
    matches_suggestion_substr(prefix, value)
}
