//! Scoreboard command argument parsers.

use std::collections::BTreeSet;

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use uuid::Uuid;

use crate::{
    command::{
        graph::{
            CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
            CommandParseErrorKind, IntRangeArgumentValue, ParsedArgument, ParsedArguments,
            ScoreHolderArgumentValue,
        },
        parsers::EntityParser,
        reader::{CommandReader, StringMode},
        requirement::CommandInputContext,
    },
    entity::Entity,
    scoreboard::ScoreHolder,
};

/// Score holder argument parser.
#[derive(Clone, Copy, Debug)]
pub struct ScoreHolderParser {
    multiple: bool,
}

impl ScoreHolderParser {
    /// Creates a parser accepting one score holder.
    #[must_use]
    pub const fn one() -> Self {
        Self { multiple: false }
    }

    /// Creates a parser accepting multiple score holders.
    #[must_use]
    pub const fn multiple() -> Self {
        Self { multiple: true }
    }
}

impl CommandArgumentParser for ScoreHolderParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_token()?;
        if raw == "*" {
            return Ok(ParsedArgument::ScoreHolders(
                ScoreHolderArgumentValue::Wildcard,
            ));
        }

        if raw.starts_with('@') {
            return parse_entity_score_holders(&raw, cursor, self.multiple, context);
        }

        Ok(ParsedArgument::ScoreHolders(
            ScoreHolderArgumentValue::Holders(vec![resolve_name_score_holder(&raw, context)]),
        ))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(
            ArgumentType::ScoreHolder {
                flags: u8::from(self.multiple),
            },
            Some(SuggestionType::AskServer),
        )
    }

    fn parsed_type(&self) -> &'static str {
        "score_holders"
    }

    fn examples(&self) -> &'static [&'static str] {
        &["Player", "0123", "*", "@e"]
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let mut suggestions = BTreeSet::new();
        if self.multiple {
            suggestions.insert("@a".to_owned());
            suggestions.insert("@e".to_owned());
            suggestions.insert("*".to_owned());
        }
        suggestions.insert("@p".to_owned());
        suggestions.insert("@r".to_owned());
        suggestions.insert("@s".to_owned());

        if let Some(server) = context.server() {
            for player in server.get_players() {
                suggestions.insert(player.scoreboard_name());
            }
            for holder in server.scoreboard.tracked_holders() {
                suggestions.insert(holder.name().to_owned());
            }
        }

        suggestions
            .into_iter()
            .filter(|suggestion| suggestion.starts_with(prefix))
            .map(SuggestionEntry::new)
            .collect()
    }
}

/// Scoreboard objective name argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct ObjectiveParser;

impl CommandArgumentParser for ObjectiveParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let value = reader.read_string(StringMode::SingleWord)?;
        Ok(ParsedArgument::ScoreboardObjective(value))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::Objective, Some(SuggestionType::AskServer))
    }

    fn parsed_type(&self) -> &'static str {
        "scoreboard_objective"
    }

    fn examples(&self) -> &'static [&'static str] {
        &["foo", "*", "012"]
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        context.server().map_or_else(Vec::new, |server| {
            server
                .scoreboard
                .objective_names()
                .into_iter()
                .filter(|name| name.starts_with(prefix))
                .map(SuggestionEntry::new)
                .collect()
        })
    }
}

/// Integer range argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct IntRangeParser;

impl CommandArgumentParser for IntRangeParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_token()?;
        parse_int_range(&raw, cursor).map(ParsedArgument::IntRange)
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::IntRange, None)
    }

    fn parsed_type(&self) -> &'static str {
        "int_range"
    }

    fn examples(&self) -> &'static [&'static str] {
        &["0..5", "0", "-5", "-100..", "..100"]
    }
}

fn parse_entity_score_holders(
    raw: &str,
    cursor: usize,
    multiple: bool,
    context: &dyn CommandInputContext,
) -> Result<ParsedArgument, CommandParseError> {
    let parser = if multiple {
        EntityParser::multiple()
    } else {
        EntityParser::one()
    };
    let mut reader = CommandReader::with_offset(raw, cursor);
    let ParsedArgument::Entities(entities) = parser.parse(&mut reader, context)? else {
        return Err(CommandParseError::new(
            CommandParseErrorKind::InvalidEntity(raw.to_owned()),
            cursor,
        ));
    };

    Ok(ParsedArgument::ScoreHolders(
        ScoreHolderArgumentValue::Holders(
            entities
                .into_iter()
                .map(|entity| ScoreHolder::new(entity.scoreboard_name()))
                .collect(),
        ),
    ))
}

fn resolve_name_score_holder(raw: &str, context: &dyn CommandInputContext) -> ScoreHolder {
    let Some(uuid) = Uuid::parse_str(raw).ok() else {
        return ScoreHolder::new(raw.to_owned());
    };
    let Some(server) = context.server() else {
        return ScoreHolder::new(raw.to_owned());
    };

    for player in server.get_players() {
        if player.uuid() == uuid {
            return ScoreHolder::new(player.scoreboard_name());
        }
    }
    for world in server.worlds.values() {
        if let Some(entity) = world.get_entity_by_uuid(&uuid) {
            return ScoreHolder::new(entity.scoreboard_name());
        }
    }

    ScoreHolder::new(raw.to_owned())
}

fn parse_int_range(raw: &str, cursor: usize) -> Result<IntRangeArgumentValue, CommandParseError> {
    if let Some((left, right)) = raw.split_once("..") {
        let min = parse_optional_range_bound(left, raw, cursor)?;
        let max = parse_optional_range_bound(right, raw, cursor)?;
        if min.is_none() && max.is_none() {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidIntegerRange(raw.to_owned()),
                cursor,
            ));
        }
        if min.zip(max).is_some_and(|(min, max)| min > max) {
            return Err(CommandParseError::new(
                CommandParseErrorKind::SwappedIntegerRange,
                cursor,
            ));
        }
        return Ok(IntRangeArgumentValue::new(min, max));
    }

    parse_range_bound(raw, raw, cursor).map(IntRangeArgumentValue::exactly)
}

fn parse_optional_range_bound(
    value: &str,
    raw: &str,
    cursor: usize,
) -> Result<Option<i32>, CommandParseError> {
    if value.is_empty() {
        return Ok(None);
    }

    parse_range_bound(value, raw, cursor).map(Some)
}

fn parse_range_bound(value: &str, raw: &str, cursor: usize) -> Result<i32, CommandParseError> {
    value.parse::<i32>().map_err(|_| {
        CommandParseError::new(
            CommandParseErrorKind::InvalidIntegerRange(raw.to_owned()),
            cursor,
        )
    })
}
