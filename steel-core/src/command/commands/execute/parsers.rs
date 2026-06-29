use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};

use crate::command::graph::{
    CommandArgumentClientParser, CommandArgumentParser, CommandParseError, CommandParseErrorKind,
    ParsedArgument, ParsedArguments,
};
use crate::command::parsers::parse_resource_identifier;
use crate::command::reader::CommandReader;
use crate::command::requirement::CommandInputContext;
use crate::command::suggestions::matches_suggestion_substr;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct AxesParser;

impl CommandArgumentParser for AxesParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let value = reader.read_token()?;
        if !is_valid_axes(&value) {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidSwizzle(value),
                cursor,
            ));
        }

        Ok(ParsedArgument::String(value))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::Swizzle, None)
    }

    fn parsed_type(&self) -> &'static str {
        "string"
    }

    fn examples(&self) -> &'static [&'static str] {
        &["x", "xz", "xyz"]
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        ["x", "y", "z", "xy", "xz", "yz", "xyz"]
            .into_iter()
            .filter(|axes| axes.starts_with(prefix))
            .map(SuggestionEntry::new)
            .collect()
    }
}

fn is_valid_axes(axes: &str) -> bool {
    if axes.is_empty() {
        return false;
    }

    let mut seen_x = false;
    let mut seen_y = false;
    let mut seen_z = false;
    for axis in axes.chars() {
        match axis {
            'x' if !seen_x => seen_x = true,
            'y' if !seen_y => seen_y = true,
            'z' if !seen_z => seen_z = true,
            _ => return false,
        }
    }
    true
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct StorageKeyParser;

impl CommandArgumentParser for StorageKeyParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_token()?;
        let Some(key) = parse_resource_identifier(&raw) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidIdentifier(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::Identifier(key))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::Identifier, Some(SuggestionType::AskServer))
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
        let Some(server) = context.server() else {
            return Vec::new();
        };

        server
            .command_storage
            .keys()
            .into_iter()
            .map(|key| key.to_string())
            .filter(|key| matches_suggestion_substr(prefix, key))
            .map(SuggestionEntry::new)
            .collect()
    }
}
