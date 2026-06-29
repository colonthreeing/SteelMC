use steel_protocol::packets::game::{
    ArgumentStringTypeBehavior, ArgumentType, SuggestionEntry, SuggestionType,
};

use super::{CommandParseError, CommandParseErrorKind, ParsedArgument, ParsedArguments};
use crate::command::{
    context::EntityAnchor,
    reader::{CommandReader, StringMode},
    requirement::{CommandInputContext, CommandSourceKind, PermissionExpr, RequirementContext},
};

/// Command argument parser metadata advertised to vanilla clients.
pub struct CommandArgumentClientParser {
    argument_type: ArgumentType,
    suggestion_type: Option<SuggestionType>,
}

impl CommandArgumentClientParser {
    /// Creates client parser metadata.
    #[must_use]
    pub const fn new(argument_type: ArgumentType, suggestion_type: Option<SuggestionType>) -> Self {
        Self {
            argument_type,
            suggestion_type,
        }
    }

    /// Returns whether the client parser consumes the rest of the command input.
    #[must_use]
    pub const fn is_greedy_phrase(&self) -> bool {
        matches!(
            &self.argument_type,
            ArgumentType::String {
                behavior: ArgumentStringTypeBehavior::GreedyPhrase,
            }
        )
    }

    /// Converts this metadata into the protocol argument tuple.
    #[must_use]
    pub fn into_protocol_argument(self) -> (ArgumentType, Option<SuggestionType>) {
        (self.argument_type, self.suggestion_type)
    }
}

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

    /// Returns the parser shape advertised to vanilla clients.
    ///
    /// This is intentionally separate from [`Self::parse`]. A Steel parser can
    /// validate stricter syntax than the vanilla client knows how to parse.
    /// For example, permission expressions are one strict server-side token,
    /// but need `greedyString` client metadata because Brigadier `word()`
    /// accepts only a hardcoded unquoted character set.
    fn client_parser(&self) -> CommandArgumentClientParser;

    /// Returns true when no command nodes can be reached after this argument.
    ///
    /// This covers parsers that consume the rest of the server input and parser
    /// metadata that makes tail nodes unreachable in the vanilla client.
    fn is_terminal_argument(&self) -> bool {
        self.client_parser().is_greedy_phrase()
    }

    /// Returns the parsed argument type stored by this parser.
    fn parsed_type(&self) -> &'static str;

    /// Returns representative inputs accepted by this parser.
    ///
    /// This mirrors Brigadier's example-based ambiguity diagnostics. The
    /// examples are not an exhaustive grammar definition.
    fn examples(&self) -> &'static [&'static str] {
        &[]
    }

    /// Returns whether this parser accepts `input` as one argument token.
    fn is_valid_input(&self, input: &str) -> bool {
        let mut reader = CommandReader::new(input);
        self.parse(&mut reader, &ParserValidationContext).is_ok()
            && (!reader.can_read() || reader.peek().is_some_and(char::is_whitespace))
    }

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

struct ParserValidationContext;

impl RequirementContext for ParserValidationContext {
    fn source_kind(&self) -> CommandSourceKind {
        CommandSourceKind::Console
    }

    fn has_permission(&self, _permission: &PermissionExpr) -> bool {
        true
    }
}

impl CommandInputContext for ParserValidationContext {}

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

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::Bool, None)
    }

    fn parsed_type(&self) -> &'static str {
        "bool"
    }

    fn examples(&self) -> &'static [&'static str] {
        &["true", "false"]
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

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::EntityAnchor, None)
    }

    fn parsed_type(&self) -> &'static str {
        "anchor"
    }

    fn examples(&self) -> &'static [&'static str] {
        &["feet", "eyes"]
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

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(
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

    fn examples(&self) -> &'static [&'static str] {
        &["0", "1", "-1"]
    }
}

/// 64-bit signed integer command argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct LongParser {
    min: Option<i64>,
    max: Option<i64>,
}

impl LongParser {
    /// Creates an unbounded long integer parser.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            min: None,
            max: None,
        }
    }

    /// Creates a bounded long integer parser.
    #[must_use]
    pub const fn bounded(min: Option<i64>, max: Option<i64>) -> Self {
        Self { min, max }
    }
}

impl CommandArgumentParser for LongParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_string(StringMode::SingleWord)?;
        let value = raw.parse::<i64>().map_err(|_| {
            CommandParseError::new(CommandParseErrorKind::InvalidLong(raw.clone()), cursor)
        })?;

        if let Some(min) = self.min
            && value < min
        {
            return Err(CommandParseError::new(
                CommandParseErrorKind::LongTooLow { value, min },
                cursor,
            ));
        }

        if let Some(max) = self.max
            && value > max
        {
            return Err(CommandParseError::new(
                CommandParseErrorKind::LongTooHigh { value, max },
                cursor,
            ));
        }

        Ok(ParsedArgument::I64(value))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(
            ArgumentType::Long {
                min: self.min,
                max: self.max,
            },
            None,
        )
    }

    fn parsed_type(&self) -> &'static str {
        "i64"
    }

    fn examples(&self) -> &'static [&'static str] {
        &["0", "1", "-1"]
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

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(
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

    fn examples(&self) -> &'static [&'static str] {
        &["0", "1.0", "-1.0"]
    }
}

/// 64-bit floating-point command argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct DoubleParser {
    min: Option<f64>,
    max: Option<f64>,
}

impl DoubleParser {
    /// Creates an unbounded double parser.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            min: None,
            max: None,
        }
    }

    /// Creates a bounded double parser.
    #[must_use]
    pub const fn bounded(min: Option<f64>, max: Option<f64>) -> Self {
        Self { min, max }
    }
}

impl CommandArgumentParser for DoubleParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_string(StringMode::SingleWord)?;
        let value = raw.parse::<f64>().map_err(|_| {
            CommandParseError::new(CommandParseErrorKind::InvalidDouble(raw), cursor)
        })?;

        if let Some(min) = self.min
            && value < min
        {
            return Err(CommandParseError::new(
                CommandParseErrorKind::DoubleTooLow { value, min },
                cursor,
            ));
        }

        if let Some(max) = self.max
            && value > max
        {
            return Err(CommandParseError::new(
                CommandParseErrorKind::DoubleTooHigh { value, max },
                cursor,
            ));
        }

        Ok(ParsedArgument::F64(value))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(
            ArgumentType::Double {
                min: self.min,
                max: self.max,
            },
            None,
        )
    }

    fn parsed_type(&self) -> &'static str {
        "f64"
    }

    fn examples(&self) -> &'static [&'static str] {
        &["0", "1.0", "-1.0"]
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

    fn client_parser(&self) -> CommandArgumentClientParser {
        let behavior = match self.mode {
            StringMode::SingleWord => ArgumentStringTypeBehavior::SingleWord,
            StringMode::QuotablePhrase => ArgumentStringTypeBehavior::QuotablePhrase,
            StringMode::GreedyPhrase => ArgumentStringTypeBehavior::GreedyPhrase,
        };

        CommandArgumentClientParser::new(ArgumentType::String { behavior }, None)
    }

    fn parsed_type(&self) -> &'static str {
        "string"
    }

    fn examples(&self) -> &'static [&'static str] {
        match self.mode {
            StringMode::SingleWord => &["word", "words_with_underscores"],
            StringMode::QuotablePhrase => &["word", "\"quoted phrase\""],
            StringMode::GreedyPhrase => &["word", "words with spaces"],
        }
    }
}
