//! Scoreboard command argument parsers.

use std::collections::BTreeSet;

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use uuid::Uuid;

use crate::{
    command::{
        error::CommandError,
        graph::{
            CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
            CommandParseErrorKind, ParsedArgument, ParsedArguments,
        },
        parsers::{
            EntityTargetArgumentValue,
            selector::{parse_entity_selector_argument, selector_argument_suggestions},
        },
        reader::{ARGUMENT_SEPARATOR, CommandReader, StringMode},
        requirement::CommandInputContext,
    },
    entity::Entity,
    scoreboard::ScoreHolder,
};

/// Score holder command argument value.
#[derive(Clone, Debug)]
pub enum ScoreHolderArgumentValue {
    /// A direct holder name, resolved against online players at execution time.
    Name(String),
    /// A UUID holder, resolved against live entities at execution time.
    Uuid {
        /// Parsed UUID.
        uuid: Uuid,
        /// Original token, used as the fallback holder name.
        raw: String,
    },
    /// Entity selector resolved to scoreboard holders at execution time.
    Selector(EntityTargetArgumentValue),
    /// Wildcard holder expansion.
    Wildcard,
}

impl ScoreHolderArgumentValue {
    /// Returns whether this argument is a wildcard.
    #[must_use]
    pub const fn is_wildcard(&self) -> bool {
        matches!(self, Self::Wildcard)
    }
}

/// Scoreboard objective name command argument value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScoreboardObjectiveName(String);

impl ScoreboardObjectiveName {
    /// Creates an objective name value.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Returns the objective name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Inclusive integer range command argument value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntRangeArgumentValue {
    min: Option<i32>,
    max: Option<i32>,
}

impl IntRangeArgumentValue {
    /// Creates an integer range.
    #[must_use]
    pub const fn new(min: Option<i32>, max: Option<i32>) -> Self {
        Self { min, max }
    }

    /// Creates an exact-value range.
    #[must_use]
    pub const fn exactly(value: i32) -> Self {
        Self {
            min: Some(value),
            max: Some(value),
        }
    }

    /// Returns whether `value` matches this range.
    #[must_use]
    pub fn matches(self, value: i32) -> bool {
        if let Some(min) = self.min
            && value < min
        {
            return false;
        }
        if let Some(max) = self.max
            && value > max
        {
            return false;
        }
        true
    }
}

/// Inclusive double range command argument value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DoubleRangeArgumentValue {
    min: Option<f64>,
    max: Option<f64>,
}

impl DoubleRangeArgumentValue {
    /// Creates a double range.
    #[must_use]
    pub const fn new(min: Option<f64>, max: Option<f64>) -> Self {
        Self { min, max }
    }

    /// Creates an exact-value range.
    #[must_use]
    pub const fn exactly(value: f64) -> Self {
        Self {
            min: Some(value),
            max: Some(value),
        }
    }

    /// Returns whether `value` matches this range.
    #[must_use]
    pub fn matches(self, value: f64) -> bool {
        if let Some(min) = self.min
            && value < min
        {
            return false;
        }
        if let Some(max) = self.max
            && value > max
        {
            return false;
        }
        true
    }
}

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
        let argument_start = reader.cursor();
        parse_int_range(reader, argument_start, cursor).map(ParsedArgument::IntRange)
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
        let argument_start = reader.cursor();
        parse_double_range(reader, argument_start, cursor).map(ParsedArgument::DoubleRange)
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
            .resolve_optional(context)?
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

fn parse_int_range(
    reader: &mut CommandReader<'_>,
    argument_start: usize,
    error_cursor: usize,
) -> Result<IntRangeArgumentValue, CommandParseError> {
    let min = read_optional_int_range_bound(reader, argument_start, error_cursor)?;
    let max = if reader.peek() == Some('.') && peek_next(reader) == Some('.') {
        let _ = reader.read();
        let _ = reader.read();
        read_optional_int_range_bound(reader, argument_start, error_cursor)?
    } else {
        min
    };

    if min.is_none() && max.is_none() {
        return Err(invalid_integer_range_error(
            reader,
            argument_start,
            error_cursor,
        ));
    }
    if min.zip(max).is_some_and(|(min, max)| min > max) {
        return Err(CommandParseError::new(
            CommandParseErrorKind::SwappedIntegerRange,
            error_cursor,
        ));
    }

    Ok(IntRangeArgumentValue::new(min, max))
}

fn parse_double_range(
    reader: &mut CommandReader<'_>,
    argument_start: usize,
    error_cursor: usize,
) -> Result<DoubleRangeArgumentValue, CommandParseError> {
    let min = read_optional_double_range_bound(reader, argument_start, error_cursor)?;
    let max = if reader.peek() == Some('.') && peek_next(reader) == Some('.') {
        let _ = reader.read();
        let _ = reader.read();
        read_optional_double_range_bound(reader, argument_start, error_cursor)?
    } else {
        min
    };

    if min.is_none() && max.is_none() {
        return Err(invalid_double_range_error(
            reader,
            argument_start,
            error_cursor,
        ));
    }
    if min.zip(max).is_some_and(|(min, max)| min > max) {
        return Err(CommandParseError::new(
            CommandParseErrorKind::SwappedDoubleRange,
            error_cursor,
        ));
    }

    Ok(DoubleRangeArgumentValue::new(min, max))
}

fn read_optional_int_range_bound(
    reader: &mut CommandReader<'_>,
    argument_start: usize,
    error_cursor: usize,
) -> Result<Option<i32>, CommandParseError> {
    let start = reader.cursor();
    read_range_number(reader);
    if reader.cursor() == start {
        return Ok(None);
    }

    reader.input()[start..reader.cursor()]
        .parse::<i32>()
        .map(Some)
        .map_err(|_| invalid_integer_range_error(reader, argument_start, error_cursor))
}

fn read_optional_double_range_bound(
    reader: &mut CommandReader<'_>,
    argument_start: usize,
    error_cursor: usize,
) -> Result<Option<f64>, CommandParseError> {
    let start = reader.cursor();
    read_range_number(reader);
    if reader.cursor() == start {
        return Ok(None);
    }

    match reader.input()[start..reader.cursor()].parse::<f64>() {
        Ok(value) if value.is_finite() => Ok(Some(value)),
        _ => Err(invalid_double_range_error(
            reader,
            argument_start,
            error_cursor,
        )),
    }
}

fn read_range_number(reader: &mut CommandReader<'_>) {
    while reader
        .peek()
        .is_some_and(|ch| is_range_number_char(ch, peek_next(reader)))
    {
        let _ = reader.read();
    }
}

fn peek_next(reader: &CommandReader<'_>) -> Option<char> {
    let mut chars = reader.remaining().chars();
    chars.next()?;
    chars.next()
}

fn is_range_number_char(ch: char, next: Option<char>) -> bool {
    ch.is_ascii_digit() || ch == '-' || (ch == '.' && next != Some('.'))
}

fn invalid_integer_range_error(
    reader: &CommandReader<'_>,
    argument_start: usize,
    error_cursor: usize,
) -> CommandParseError {
    CommandParseError::new(
        CommandParseErrorKind::InvalidIntegerRange(range_error_raw(reader, argument_start)),
        error_cursor,
    )
}

fn invalid_double_range_error(
    reader: &CommandReader<'_>,
    argument_start: usize,
    error_cursor: usize,
) -> CommandParseError {
    CommandParseError::new(
        CommandParseErrorKind::InvalidDoubleRange(range_error_raw(reader, argument_start)),
        error_cursor,
    )
}

fn range_error_raw(reader: &CommandReader<'_>, argument_start: usize) -> String {
    let end = if reader.cursor() == argument_start {
        next_argument_separator(reader, argument_start)
    } else {
        reader.cursor()
    };
    reader.input()[argument_start..end].trim_end().to_owned()
}

fn next_argument_separator(reader: &CommandReader<'_>, start: usize) -> usize {
    let Some((offset, _)) = reader.input()[start..]
        .char_indices()
        .find(|(_, ch)| *ch == ARGUMENT_SEPARATOR)
    else {
        return reader.input().len();
    };

    start + offset
}
