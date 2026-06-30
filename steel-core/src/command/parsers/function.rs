//! Command function argument parser.

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};

use crate::command::{
    graph::{
        CommandArgumentClientParser, CommandArgumentParser, CommandFunctionArgumentValue,
        CommandParseError, CommandParseErrorKind, ParsedArgument, ParsedArguments,
    },
    parsers::parse_resource_identifier,
    reader::CommandReader,
    requirement::CommandInputContext,
    suggestions::matches_suggestion_substr,
};

/// Vanilla command function resource-or-tag argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct CommandFunctionParser;

impl CommandArgumentParser for CommandFunctionParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let start = reader.absolute_cursor();
        let is_tag = reader.peek() == Some('#');
        if is_tag {
            reader.read();
        }

        let raw = reader.read_token()?;
        let Some(id) = parse_resource_identifier(&raw) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidCommandFunction(raw),
                start,
            ));
        };

        Ok(ParsedArgument::CommandFunction(if is_tag {
            CommandFunctionArgumentValue::Tag(id)
        } else {
            CommandFunctionArgumentValue::Function(id)
        }))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::Function, Some(SuggestionType::AskServer))
    }

    fn parsed_type(&self) -> &'static str {
        "command_function"
    }

    fn examples(&self) -> &'static [&'static str] {
        &["foo", "foo:bar", "#foo"]
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
        let registry = server.command_functions.read();
        if let Some(tag_prefix) = prefix.strip_prefix('#') {
            let stripped_tag_prefix = tag_prefix.strip_prefix("minecraft:").unwrap_or(tag_prefix);
            return registry
                .tag_ids()
                .into_iter()
                .filter(|id| {
                    let key = id.to_string();
                    let text = key.strip_prefix("minecraft:").unwrap_or(&key);
                    matches_suggestion_substr(stripped_tag_prefix, text)
                })
                .map(|id| SuggestionEntry::new(format!("#{id}")))
                .collect();
        }

        let stripped_prefix = prefix.strip_prefix("minecraft:").unwrap_or(prefix);
        registry
            .function_ids()
            .into_iter()
            .filter(|id| {
                let key = id.to_string();
                let text = key.strip_prefix("minecraft:").unwrap_or(&key);
                matches_suggestion_substr(stripped_prefix, text)
            })
            .map(|id| SuggestionEntry::new(id.to_string()))
            .collect()
    }
}
