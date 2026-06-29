//! Domain and world command argument parsers.

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use steel_utils::Identifier;

use crate::command::{
    graph::{
        CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
        CommandParseErrorKind, ParsedArgument, ParsedArguments,
    },
    reader::CommandReader,
    requirement::CommandInputContext,
};

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

impl CommandArgumentParser for WorldParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_token()?;
        let Some(server) = context.server() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("server"),
                cursor,
            ));
        };

        if let Some(world) = raw
            .parse::<Identifier>()
            .ok()
            .and_then(|key| server.worlds.get(&key).cloned())
        {
            return Ok(ParsedArgument::World(world));
        }

        let Some(current_world) = context.world() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("world"),
                cursor,
            ));
        };

        let key = Identifier::new(current_world.domain().to_owned(), raw.clone());
        let Some(world) = server.worlds.get(&key).cloned() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidWorld(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::World(world))
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
