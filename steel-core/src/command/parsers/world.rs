//! Domain and world command argument parsers.

use std::{borrow::Cow, fmt, sync::Arc};

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use steel_utils::Identifier;
use text_components::{TextComponent, translation::TranslatedMessage};

use crate::command::{
    context::CommandContext,
    error::CommandError,
    graph::{
        CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
        CommandParseErrorKind, ParsedArgument, ParsedArguments,
    },
    reader::CommandReader,
    requirement::CommandInputContext,
};
use crate::world::World;

use super::parse_resource_identifier;

/// Configured domain name argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct DomainParser;

impl CommandArgumentParser for DomainParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let domain = reader.read_token()?;
        let Some(server) = context.server() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("server"),
                cursor,
            ));
        };

        if !server.worlds.has_domain(&domain) {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidDomain(domain),
                cursor,
            ));
        }

        Ok(ParsedArgument::String(domain))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::Identifier, Some(SuggestionType::AskServer))
    }

    fn parsed_type(&self) -> &'static str {
        "string"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let Some(server) = context.server() else {
            return Vec::new();
        };

        server
            .worlds
            .domain_names()
            .filter(|domain| domain.starts_with(prefix))
            .map(SuggestionEntry::new)
            .collect()
    }
}

/// Loaded world argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct WorldParser;

/// Parsed loaded-world argument.
///
/// Vanilla parses dimensions as identifiers and resolves them against the
/// server when the command executes. Steel keeps that shape, while also
/// supporting a relative world name resolved inside the current domain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorldArgumentValue {
    /// Fully-qualified world key.
    Key(Identifier),
    /// World name relative to the executing context's current domain.
    Relative(String),
}

impl WorldArgumentValue {
    fn parse(raw: String, cursor: usize) -> Result<Self, CommandParseError> {
        if raw.contains(':') {
            let key = parse_resource_identifier(&raw).ok_or_else(|| {
                CommandParseError::new(CommandParseErrorKind::InvalidWorld(raw.clone()), cursor)
            })?;
            return Ok(Self::Key(key));
        }
        if !Identifier::validate_path(&raw) {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidWorld(raw),
                cursor,
            ));
        }
        Ok(Self::Relative(raw))
    }

    /// Resolves this parsed argument against live server state.
    ///
    /// # Errors
    ///
    /// Returns a command failure when the target world is not loaded.
    pub fn resolve(&self, context: &CommandContext) -> Result<Arc<World>, CommandError> {
        let key = match self {
            Self::Key(key) => key.clone(),
            Self::Relative(path) => {
                Identifier::new(context.world.domain().to_owned(), path.clone())
            }
        };

        context
            .server
            .worlds
            .get(&key)
            .cloned()
            .ok_or_else(|| invalid_world_error(&self.to_string()))
    }
}

impl fmt::Display for WorldArgumentValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(key) => write!(f, "{key}"),
            Self::Relative(path) => f.write_str(path),
        }
    }
}

fn invalid_world_error(value: &str) -> CommandError {
    CommandError::failure(TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed("argument.dimension.invalid"),
        fallback: None,
        args: Some(Box::new([TextComponent::from(value.to_owned())])),
    }))
}

impl CommandArgumentParser for WorldParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_token()?;
        Ok(ParsedArgument::World(WorldArgumentValue::parse(
            raw, cursor,
        )?))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::Dimension, Some(SuggestionType::AskServer))
    }

    fn parsed_type(&self) -> &'static str {
        "world"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let (Some(server), Some(current_world)) = (context.server(), context.world()) else {
            return Vec::new();
        };
        let current_domain = current_world.domain();

        let mut suggestions = server
            .worlds
            .keys()
            .map(|id| SuggestionEntry::new(id.to_string()))
            .collect::<Vec<_>>();

        for id in server.worlds.keys() {
            if id.namespace.as_ref() == current_domain {
                let path = id.path.as_ref();
                if !suggestions.iter().any(|suggestion| suggestion.text == path) {
                    suggestions.push(SuggestionEntry::new(path));
                }
            }
        }

        suggestions.retain(|suggestion| suggestion.text.starts_with(prefix));
        suggestions
    }
}
