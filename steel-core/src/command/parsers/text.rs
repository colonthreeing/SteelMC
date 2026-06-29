//! Text and time command argument parsers.

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry};
use text_components::TextComponent;

use crate::command::{
    graph::{
        CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
        CommandParseErrorKind, ParsedArgument, ParsedArguments,
    },
    reader::{CommandReader, StringMode},
    requirement::CommandInputContext,
};

/// Text component argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct ComponentParser;

impl CommandArgumentParser for ComponentParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_string(StringMode::GreedyPhrase)?;
        let component = TextComponent::from_snbt(&raw).map_err(|error| {
            CommandParseError::new(
                CommandParseErrorKind::InvalidComponent(error.to_string()),
                cursor,
            )
        })?;

        Ok(ParsedArgument::Component(Box::new(component)))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::Component, None)
    }

    fn parsed_type(&self) -> &'static str {
        "component"
    }
}

/// Time argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct TimeParser;

impl CommandArgumentParser for TimeParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let value = reader.read_string(StringMode::SingleWord)?;

        let (number, unit) = value
            .find(char::is_alphabetic)
            .map_or((value.as_str(), "t"), |pos| (&value[..pos], &value[pos..]));

        let Ok(number) = number.parse::<f32>() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidTime(value),
                cursor,
            ));
        };

        if number < 0.0 {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidTime(value),
                cursor,
            ));
        }

        let ticks = match unit {
            "d" => number * 24_000.0,
            "s" => number * 20.0,
            "t" => number,
            _ => {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidTime(value),
                    cursor,
                ));
            }
        };

        Ok(ParsedArgument::I32(ticks.round() as i32))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::Time { min: 0 }, None)
    }

    fn parsed_type(&self) -> &'static str {
        "i32"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let has_unit = prefix.chars().any(char::is_alphabetic);
        if prefix.is_empty() || has_unit {
            return Vec::new();
        }

        ["d", "s", "t"]
            .into_iter()
            .map(|unit| SuggestionEntry::new(format!("{prefix}{unit}")))
            .collect()
    }
}
