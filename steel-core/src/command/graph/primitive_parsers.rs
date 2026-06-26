use steel_protocol::packets::game::{
    ArgumentStringTypeBehavior, ArgumentType, SuggestionEntry, SuggestionType,
};

use super::{CommandParseError, CommandParseErrorKind, ParsedArgument, ParsedArguments};
use crate::command::{
    context::EntityAnchor,
    reader::{CommandReader, StringMode},
    requirement::CommandInputContext,
};

/// Dynamic command argument parser.
pub trait CommandArgumentParser: Send + Sync {
    /// Parses one argument at the reader cursor.
    ///
    /// # Errors
    ///
    /// Returns a structured parse error when the argument is invalid.
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError>;

    /// Returns protocol parser metadata for this argument.
    fn usage(&self) -> (ArgumentType, Option<SuggestionType>);

    /// Returns the parsed argument type stored by this parser.
    fn parsed_type(&self) -> &'static str;

    /// Returns suggestions for the current argument token.
    fn suggest(
        &self,
        _prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        Vec::new()
    }
}

/// Boolean command argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct BoolParser;

impl CommandArgumentParser for BoolParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let value = reader.read_string(StringMode::SingleWord)?;

        match value.as_str() {
            "true" => Ok(ParsedArgument::Bool(true)),
            "false" => Ok(ParsedArgument::Bool(false)),
            _ => Err(CommandParseError::new(
                CommandParseErrorKind::InvalidBool(value),
                cursor,
            )),
        }
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (ArgumentType::Bool, None)
    }

    fn parsed_type(&self) -> &'static str {
        "bool"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        ["true", "false"]
            .into_iter()
            .filter(|suggestion| suggestion.starts_with(prefix))
            .map(SuggestionEntry::new)
            .collect()
    }
}

/// Entity anchor argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct AnchorParser;

impl CommandArgumentParser for AnchorParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let value = reader.read_string(StringMode::SingleWord)?;

        match value.as_str() {
            "feet" => Ok(ParsedArgument::Anchor(EntityAnchor::Feet)),
            "eyes" => Ok(ParsedArgument::Anchor(EntityAnchor::Eyes)),
            _ => Err(CommandParseError::new(
                CommandParseErrorKind::InvalidAnchor(value),
                cursor,
            )),
        }
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (ArgumentType::EntityAnchor, None)
    }

    fn parsed_type(&self) -> &'static str {
        "anchor"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        ["feet", "eyes"]
            .into_iter()
            .filter(|suggestion| suggestion.starts_with(prefix))
            .map(SuggestionEntry::new)
            .collect()
    }
}

/// 32-bit signed integer command argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct IntegerParser {
    min: Option<i32>,
    max: Option<i32>,
}

impl IntegerParser {
    /// Creates an unbounded integer parser.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            min: None,
            max: None,
        }
    }

    /// Creates a bounded integer parser.
    #[must_use]
    pub const fn bounded(min: Option<i32>, max: Option<i32>) -> Self {
        Self { min, max }
    }
}

impl CommandArgumentParser for IntegerParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_string(StringMode::SingleWord)?;
        let value = raw.parse::<i32>().map_err(|_| {
            CommandParseError::new(CommandParseErrorKind::InvalidInteger(raw.clone()), cursor)
        })?;

        if let Some(min) = self.min
            && value < min
        {
            return Err(CommandParseError::new(
                CommandParseErrorKind::IntegerTooLow { value, min },
                cursor,
            ));
        }

        if let Some(max) = self.max
            && value > max
        {
            return Err(CommandParseError::new(
                CommandParseErrorKind::IntegerTooHigh { value, max },
                cursor,
            ));
        }

        Ok(ParsedArgument::I32(value))
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (
            ArgumentType::Integer {
                min: self.min,
                max: self.max,
            },
            None,
        )
    }

    fn parsed_type(&self) -> &'static str {
        "i32"
    }
}

/// 32-bit floating-point command argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct FloatParser {
    min: Option<f32>,
    max: Option<f32>,
}

impl FloatParser {
    /// Creates an unbounded float parser.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            min: None,
            max: None,
        }
    }

    /// Creates a bounded float parser.
    #[must_use]
    pub const fn bounded(min: Option<f32>, max: Option<f32>) -> Self {
        Self { min, max }
    }
}

impl CommandArgumentParser for FloatParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_string(StringMode::SingleWord)?;
        let value = raw.parse::<f32>().map_err(|_| {
            CommandParseError::new(CommandParseErrorKind::InvalidFloat(raw), cursor)
        })?;

        if let Some(min) = self.min
            && value < min
        {
            return Err(CommandParseError::new(
                CommandParseErrorKind::FloatTooLow { value, min },
                cursor,
            ));
        }

        if let Some(max) = self.max
            && value > max
        {
            return Err(CommandParseError::new(
                CommandParseErrorKind::FloatTooHigh { value, max },
                cursor,
            ));
        }

        Ok(ParsedArgument::F32(value))
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (
            ArgumentType::Float {
                min: self.min,
                max: self.max,
            },
            None,
        )
    }

    fn parsed_type(&self) -> &'static str {
        "f32"
    }
}

/// String command argument parser.
#[derive(Clone, Copy, Debug)]
pub struct StringParser {
    mode: StringMode,
}

impl StringParser {
    /// Creates a string parser with `mode`.
    #[must_use]
    pub const fn new(mode: StringMode) -> Self {
        Self { mode }
    }
}

impl CommandArgumentParser for StringParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        reader.read_string(self.mode).map(ParsedArgument::String)
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        let behavior = match self.mode {
            StringMode::SingleWord => ArgumentStringTypeBehavior::SingleWord,
            StringMode::QuotablePhrase => ArgumentStringTypeBehavior::QuotablePhrase,
            StringMode::GreedyPhrase => ArgumentStringTypeBehavior::GreedyPhrase,
        };

        (ArgumentType::String { behavior }, None)
    }

    fn parsed_type(&self) -> &'static str {
        "string"
    }
}
