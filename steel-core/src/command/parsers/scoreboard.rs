//! Scoreboard command argument parsers.

use std::collections::BTreeSet;

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use uuid::Uuid;

use crate::{
    command::{
        error::CommandError,
        graph::{
            CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
            CommandParseErrorKind, DoubleRangeArgumentValue, IntRangeArgumentValue, ParsedArgument,
            ParsedArguments, ScoreHolderArgumentValue,
        },
        parsers::{
            EntityTargetArgumentValue,
            selector::{parse_entity_selector_argument, selector_argument_suggestions},
        },
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
        if reader.peek() == Some('@') {
            let input = reader.input().to_owned();
            let (selector, cursor) =
                parse_entity_selector_argument(reader, context, !self.multiple)?;
            return Ok(ParsedArgument::ScoreHolders(
                ScoreHolderArgumentValue::Selector(EntityTargetArgumentValue::new(
                    selector,
                    cursor,
                    input,
                    !self.multiple,
                )),
            ));
        }

        let raw = reader.read_token()?;
        let value = if raw == "*" {
            return Ok(ParsedArgument::ScoreHolders(
                ScoreHolderArgumentValue::Wildcard,
            ));
        } else if let Ok(uuid) = Uuid::parse_str(&raw) {
            ScoreHolderArgumentValue::Uuid { uuid, raw }
        } else {
            ScoreHolderArgumentValue::Name(raw)
        };

        Ok(ParsedArgument::ScoreHolders(value))
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
            suggestions.extend(("*".starts_with(prefix)).then_some("*".to_owned()));
        }
        suggestions.extend(selector_argument_suggestions(
            prefix,
            false,
            !self.multiple,
            context,
        ));

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

/// Double range argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct DoubleRangeParser;

impl CommandArgumentParser for DoubleRangeParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_token()?;
        parse_double_range(&raw, cursor).map(ParsedArgument::DoubleRange)
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::FloatRange, None)
    }

    fn parsed_type(&self) -> &'static str {
        "double_range"
    }

    fn examples(&self) -> &'static [&'static str] {
        &["0..5", "0", "-5.0", "-100.0..", "..100.0"]
    }
}

/// Wildcard behavior for resolving score-holder arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScoreHolderWildcardExpansion {
    /// Wildcard expands to an empty collection.
    Empty,
    /// Wildcard expands to all scoreboard-tracked holders.
    TrackedHolders,
}

pub(crate) fn resolve_score_holders(
    value: ScoreHolderArgumentValue,
    context: &dyn CommandInputContext,
    wildcard: ScoreHolderWildcardExpansion,
) -> Result<Vec<ScoreHolder>, CommandError> {
    Ok(match value {
        ScoreHolderArgumentValue::Name(raw) => vec![resolve_name_score_holder(&raw, context)],
        ScoreHolderArgumentValue::Uuid { uuid, raw } => {
            resolve_uuid_score_holders(uuid, &raw, context)
        }
        ScoreHolderArgumentValue::Selector(selector) => selector
            .resolve(context)?
            .into_iter()
            .map(|entity| ScoreHolder::new(entity.scoreboard_name()))
            .collect(),
        ScoreHolderArgumentValue::Wildcard => match wildcard {
            ScoreHolderWildcardExpansion::Empty => Vec::new(),
            ScoreHolderWildcardExpansion::TrackedHolders => context
                .server()
                .map_or_else(Vec::new, |server| server.scoreboard.tracked_holders()),
        },
    })
}

fn resolve_name_score_holder(raw: &str, context: &dyn CommandInputContext) -> ScoreHolder {
    if raw.starts_with('#') {
        return ScoreHolder::new(raw.to_owned());
    }
    context
        .server()
        .and_then(|server| {
            server
                .get_players()
                .into_iter()
                .find(|player| player.gameprofile.name.eq_ignore_ascii_case(raw))
        })
        .map_or_else(
            || ScoreHolder::new(raw.to_owned()),
            |player| ScoreHolder::new(player.scoreboard_name()),
        )
}

fn resolve_uuid_score_holders(
    uuid: Uuid,
    raw: &str,
    context: &dyn CommandInputContext,
) -> Vec<ScoreHolder> {
    let Some(server) = context.server() else {
        return vec![ScoreHolder::new(raw.to_owned())];
    };
    let holders = server
        .worlds
        .values()
        .filter_map(|world| world.get_entity_by_uuid(&uuid))
        .map(|entity| ScoreHolder::new(entity.scoreboard_name()))
        .collect::<Vec<_>>();
    if !holders.is_empty() {
        return holders;
    }

    server
        .get_players()
        .into_iter()
        .find(|player| player.uuid() == uuid)
        .map_or_else(
            || vec![ScoreHolder::new(raw.to_owned())],
            |player| vec![ScoreHolder::new(player.scoreboard_name())],
        )
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

fn parse_double_range(
    raw: &str,
    cursor: usize,
) -> Result<DoubleRangeArgumentValue, CommandParseError> {
    if let Some((left, right)) = raw.split_once("..") {
        let min = parse_optional_double_range_bound(left, raw, cursor)?;
        let max = parse_optional_double_range_bound(right, raw, cursor)?;
        if min.is_none() && max.is_none() {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidDoubleRange(raw.to_owned()),
                cursor,
            ));
        }
        if min.zip(max).is_some_and(|(min, max)| min > max) {
            return Err(CommandParseError::new(
                CommandParseErrorKind::SwappedDoubleRange,
                cursor,
            ));
        }
        return Ok(DoubleRangeArgumentValue::new(min, max));
    }

    parse_double_range_bound(raw, raw, cursor).map(DoubleRangeArgumentValue::exactly)
}

fn parse_optional_double_range_bound(
    value: &str,
    raw: &str,
    cursor: usize,
) -> Result<Option<f64>, CommandParseError> {
    if value.is_empty() {
        return Ok(None);
    }

    parse_double_range_bound(value, raw, cursor).map(Some)
}

fn parse_double_range_bound(
    value: &str,
    raw: &str,
    cursor: usize,
) -> Result<f64, CommandParseError> {
    match value.parse::<f64>() {
        Ok(value) if value.is_finite() => Ok(value),
        _ => Err(CommandParseError::new(
            CommandParseErrorKind::InvalidDoubleRange(raw.to_owned()),
            cursor,
        )),
    }
}
