//! Cursor-based command input reader.

use crate::command::graph::{CommandParseError, CommandParseErrorKind};

/// Brigadier command node separator.
pub const ARGUMENT_SEPARATOR: char = ' ';

/// The string parsing mode used by Brigadier string arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringMode {
    /// Reads a single unquoted word.
    SingleWord,
    /// Reads either a single word or a quoted phrase.
    QuotablePhrase,
    /// Reads the rest of the input.
    GreedyPhrase,
}

/// Reader for command input that tracks a byte cursor plus an absolute offset.
///
/// The cursor is byte-based for Rust slicing. Use [`Self::utf16_cursor`] when a
/// protocol-facing cursor is needed, since vanilla clients use Java string
/// indices.
#[derive(Clone, Debug)]
pub struct CommandReader<'a> {
    input: &'a str,
    cursor: usize,
    cursor_offset: usize,
}

impl<'a> CommandReader<'a> {
    /// Creates a reader at the start of `input`.
    #[must_use]
    pub const fn new(input: &'a str) -> Self {
        Self {
            input,
            cursor: 0,
            cursor_offset: 0,
        }
    }

    /// Creates a reader whose reported absolute cursor is offset by `cursor_offset`.
    #[must_use]
    pub const fn with_offset(input: &'a str, cursor_offset: usize) -> Self {
        Self {
            input,
            cursor: 0,
            cursor_offset,
        }
    }

    /// Returns the input this reader is consuming.
    #[must_use]
    pub const fn input(&self) -> &'a str {
        self.input
    }

    /// Returns the current byte cursor relative to this reader's input.
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// Returns the current byte cursor including this reader's absolute offset.
    #[must_use]
    pub const fn absolute_cursor(&self) -> usize {
        self.cursor_offset + self.cursor
    }

    /// Returns the current cursor as a UTF-16 code-unit index, matching Java strings.
    #[must_use]
    pub fn utf16_cursor(&self) -> usize {
        self.input[..self.cursor].encode_utf16().count()
    }

    /// Returns the current cursor as an absolute UTF-16 code-unit index.
    #[must_use]
    pub fn absolute_utf16_cursor(&self) -> usize {
        self.cursor_offset + self.utf16_cursor()
    }

    /// Returns true when there is input left to read.
    #[must_use]
    pub const fn can_read(&self) -> bool {
        self.cursor < self.input.len()
    }

    /// Returns the unread input.
    #[must_use]
    pub fn remaining(&self) -> &'a str {
        &self.input[self.cursor..]
    }

    /// Returns the next character without advancing.
    #[must_use]
    pub fn peek(&self) -> Option<char> {
        self.remaining().chars().next()
    }

    /// Reads one character and advances the cursor.
    pub fn read(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.cursor += ch.len_utf8();
        Some(ch)
    }

    /// Skips whitespace from the current cursor.
    pub fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.read();
        }
    }

    /// Returns whether the next character is Brigadier's command node separator.
    #[must_use]
    pub fn is_argument_separator(&self) -> bool {
        self.peek() == Some(ARGUMENT_SEPARATOR)
    }

    /// Requires and consumes exactly one Brigadier command node separator.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the current character is not the separator.
    pub fn expect_argument_separator(&mut self) -> Result<(), CommandParseError> {
        if !self.is_argument_separator() {
            return Err(CommandParseError::new(
                CommandParseErrorKind::ExpectedWhitespace,
                self.absolute_cursor(),
            ));
        }

        self.read();
        Ok(())
    }

    /// Requires at least one whitespace character, then skips the full whitespace run.
    ///
    /// Use this for parser-internal whitespace, not graph node separation.
    ///
    /// # Errors
    ///
    /// Returns a parse error when the current character is not whitespace.
    pub fn expect_whitespace(&mut self) -> Result<(), CommandParseError> {
        if !self.peek().is_some_and(char::is_whitespace) {
            return Err(CommandParseError::new(
                CommandParseErrorKind::ExpectedWhitespace,
                self.absolute_cursor(),
            ));
        }

        self.skip_whitespace();
        Ok(())
    }

    /// Reads a string according to `mode`.
    ///
    /// # Errors
    ///
    /// Returns a parse error if the requested string is missing or malformed.
    pub fn read_string(&mut self, mode: StringMode) -> Result<String, CommandParseError> {
        match mode {
            StringMode::SingleWord => self.read_unquoted_string(),
            StringMode::QuotablePhrase => {
                if self.peek().is_some_and(is_quoted_string_start) {
                    self.read_quoted_string()
                } else {
                    self.read_unquoted_string()
                }
            }
            StringMode::GreedyPhrase => {
                let value = self.remaining().to_owned();
                self.cursor = self.input.len();
                Ok(value)
            }
        }
    }

    /// Reads a 32-bit signed integer with Brigadier's numeric scanner.
    ///
    /// # Errors
    ///
    /// Returns a parse error when no integer is present or the scanned number is invalid.
    pub fn read_i32(&mut self) -> Result<i32, CommandParseError> {
        let cursor = self.absolute_cursor();
        let raw = self.read_number_string(CommandParseErrorKind::ExpectedInteger)?;
        raw.parse::<i32>().map_err(|_| {
            self.cursor = cursor - self.cursor_offset;
            CommandParseError::new(CommandParseErrorKind::InvalidInteger(raw), cursor)
        })
    }

    /// Reads a 64-bit signed integer with Brigadier's numeric scanner.
    ///
    /// # Errors
    ///
    /// Returns a parse error when no long integer is present or the scanned number is invalid.
    pub fn read_i64(&mut self) -> Result<i64, CommandParseError> {
        let cursor = self.absolute_cursor();
        let raw = self.read_number_string(CommandParseErrorKind::ExpectedLong)?;
        raw.parse::<i64>().map_err(|_| {
            self.cursor = cursor - self.cursor_offset;
            CommandParseError::new(CommandParseErrorKind::InvalidLong(raw), cursor)
        })
    }

    /// Reads a 32-bit float with Brigadier's numeric scanner.
    ///
    /// # Errors
    ///
    /// Returns a parse error when no float is present or the scanned number is invalid.
    pub fn read_f32(&mut self) -> Result<f32, CommandParseError> {
        let cursor = self.absolute_cursor();
        let raw = self.read_number_string(CommandParseErrorKind::ExpectedFloat)?;
        raw.parse::<f32>().map_err(|_| {
            self.cursor = cursor - self.cursor_offset;
            CommandParseError::new(CommandParseErrorKind::InvalidFloat(raw), cursor)
        })
    }

    /// Reads a 64-bit float with Brigadier's numeric scanner.
    ///
    /// # Errors
    ///
    /// Returns a parse error when no double is present or the scanned number is invalid.
    pub fn read_f64(&mut self) -> Result<f64, CommandParseError> {
        let cursor = self.absolute_cursor();
        let raw = self.read_number_string(CommandParseErrorKind::ExpectedDouble)?;
        raw.parse::<f64>().map_err(|_| {
            self.cursor = cursor - self.cursor_offset;
            CommandParseError::new(CommandParseErrorKind::InvalidDouble(raw), cursor)
        })
    }

    fn read_number_string(
        &mut self,
        expected: CommandParseErrorKind,
    ) -> Result<String, CommandParseError> {
        let start = self.cursor;
        while self.peek().is_some_and(is_allowed_number) {
            self.read();
        }

        if self.cursor == start {
            return Err(CommandParseError::new(expected, self.absolute_cursor()));
        }

        Ok(self.input[start..self.cursor].to_owned())
    }

    /// Reads one whitespace-delimited token.
    ///
    /// Use this for custom server-side argument parsers whose token syntax is
    /// not Brigadier's `StringArgumentType.word()` syntax.
    ///
    /// # Errors
    ///
    /// Returns a parse error if there is no token at the current cursor.
    pub fn read_token(&mut self) -> Result<String, CommandParseError> {
        self.read_while(|ch| !ch.is_whitespace())
    }

    /// Reads a literal if it appears at the current cursor and is followed by a boundary.
    pub fn read_literal(&mut self, literal: &str) -> bool {
        let Some(remaining) = self.remaining().strip_prefix(literal) else {
            return false;
        };
        if remaining
            .chars()
            .next()
            .is_some_and(|ch| ch != ARGUMENT_SEPARATOR)
        {
            return false;
        }

        self.cursor += literal.len();
        true
    }

    fn read_unquoted_string(&mut self) -> Result<String, CommandParseError> {
        self.read_while(is_allowed_in_unquoted_string)
    }

    fn read_while(&mut self, allowed: impl Fn(char) -> bool) -> Result<String, CommandParseError> {
        let start = self.cursor;
        while self.peek().is_some_and(&allowed) {
            self.read();
        }

        if self.cursor == start {
            return Err(CommandParseError::new(
                CommandParseErrorKind::ExpectedArgument,
                self.absolute_cursor(),
            ));
        }

        Ok(self.input[start..self.cursor].to_owned())
    }

    fn read_quoted_string(&mut self) -> Result<String, CommandParseError> {
        let Some(terminator) = self.peek().filter(|ch| is_quoted_string_start(*ch)) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::ExpectedArgument,
                self.absolute_cursor(),
            ));
        };
        let quote_cursor = self.absolute_cursor();
        debug_assert_eq!(self.read(), Some(terminator));

        let mut value = String::new();
        while let Some(ch) = self.read() {
            match ch {
                ch if ch == terminator => return Ok(value),
                '\\' => value.push(self.read_escaped_char(terminator)?),
                _ => value.push(ch),
            }
        }

        Err(CommandParseError::new(
            CommandParseErrorKind::UnclosedQuote,
            quote_cursor,
        ))
    }

    fn read_escaped_char(&mut self, terminator: char) -> Result<char, CommandParseError> {
        let escape_cursor = self.absolute_cursor();
        match self.read() {
            Some(ch) if ch == '\\' || ch == terminator => Ok(ch),
            Some(ch) => Err(CommandParseError::new(
                CommandParseErrorKind::InvalidEscape(ch),
                escape_cursor,
            )),
            None => Err(CommandParseError::new(
                CommandParseErrorKind::UnclosedQuote,
                escape_cursor,
            )),
        }
    }
}

fn is_quoted_string_start(ch: char) -> bool {
    matches!(ch, '"' | '\'')
}

fn is_allowed_in_unquoted_string(ch: char) -> bool {
    matches!(ch, '0'..='9' | 'A'..='Z' | 'a'..='z' | '_' | '-' | '.' | '+')
}

fn is_allowed_number(ch: char) -> bool {
    matches!(ch, '0'..='9' | '-' | '.')
}

#[cfg(test)]
mod tests {
    use super::{CommandReader, StringMode};
    use crate::command::graph::CommandParseErrorKind;

    #[test]
    fn single_word_stops_at_whitespace() {
        let mut reader = CommandReader::new("hello world");

        assert_eq!(
            reader
                .read_string(StringMode::SingleWord)
                .expect("single word parses"),
            "hello"
        );
        assert_eq!(reader.cursor(), 5);
    }

    #[test]
    fn single_word_uses_brigadier_unquoted_characters() {
        let mut reader = CommandReader::new("plugin:region");

        assert_eq!(
            reader
                .read_string(StringMode::SingleWord)
                .expect("single word parses"),
            "plugin"
        );
        assert_eq!(reader.remaining(), ":region");
    }

    #[test]
    fn token_reads_until_whitespace() {
        let mut reader = CommandReader::new("plugin:region{world=spawn} tail");

        assert_eq!(
            reader.read_token().expect("token parses"),
            "plugin:region{world=spawn}"
        );
        assert_eq!(reader.remaining(), " tail");
    }

    #[test]
    fn argument_separator_consumes_one_ascii_space() {
        let mut reader = CommandReader::new("  tail");

        reader
            .expect_argument_separator()
            .expect("space is the graph separator");

        assert_eq!(reader.remaining(), " tail");

        let mut reader = CommandReader::new("\ttail");
        let error = reader
            .expect_argument_separator()
            .expect_err("tab is not the graph separator");
        assert_eq!(error.kind(), &CommandParseErrorKind::ExpectedWhitespace);
        assert_eq!(error.cursor(), 0);
    }

    #[test]
    fn numeric_scanners_use_brigadier_number_characters() {
        let mut reader = CommandReader::new("1e3");
        assert_eq!(reader.read_i32().expect("integer parses"), 1);
        assert_eq!(reader.remaining(), "e3");

        let mut reader = CommandReader::new("1e3");
        assert_eq!(reader.read_f64().expect("double parses"), 1.0);
        assert_eq!(reader.remaining(), "e3");
    }

    #[test]
    fn numeric_scanners_reject_leading_plus_without_consuming() {
        let mut reader = CommandReader::new("+1");
        let error = reader
            .read_i32()
            .expect_err("leading plus is not a Brigadier number character");

        assert_eq!(error.kind(), &CommandParseErrorKind::ExpectedInteger);
        assert_eq!(error.cursor(), 0);
        assert_eq!(reader.remaining(), "+1");
    }

    #[test]
    fn numeric_scanners_reset_after_invalid_scanned_number() {
        let mut reader = CommandReader::new("--1");
        let error = reader
            .read_i32()
            .expect_err("invalid scanned integer should fail");

        assert_eq!(
            error.kind(),
            &CommandParseErrorKind::InvalidInteger("--1".to_owned())
        );
        assert_eq!(error.cursor(), 0);
        assert_eq!(reader.remaining(), "--1");
    }

    #[test]
    fn literal_requires_boundary() {
        let mut reader = CommandReader::new("root:tail");

        assert!(!reader.read_literal("root"));
        assert_eq!(reader.cursor(), 0);

        let mut reader = CommandReader::new("root\ttail");
        assert!(!reader.read_literal("root"));
        assert_eq!(reader.cursor(), 0);

        let mut reader = CommandReader::new("root tail");
        assert!(reader.read_literal("root"));
        assert_eq!(reader.remaining(), " tail");
    }

    #[test]
    fn quoted_phrase_handles_escapes() {
        let mut reader = CommandReader::new("\"hello \\\"world\\\"\" tail");

        assert_eq!(
            reader
                .read_string(StringMode::QuotablePhrase)
                .expect("quoted phrase parses"),
            "hello \"world\""
        );
        assert_eq!(reader.remaining(), " tail");
    }

    #[test]
    fn quoted_phrase_handles_single_quotes() {
        let mut reader = CommandReader::new("'hello \\'world\\'' tail");

        assert_eq!(
            reader
                .read_string(StringMode::QuotablePhrase)
                .expect("quoted phrase parses"),
            "hello 'world'"
        );
        assert_eq!(reader.remaining(), " tail");
    }

    #[test]
    fn unclosed_quote_reports_quote_cursor() {
        let mut reader = CommandReader::with_offset("\"unterminated", 1);

        let error = reader
            .read_string(StringMode::QuotablePhrase)
            .expect_err("quote should be unclosed");

        assert_eq!(error.kind(), &CommandParseErrorKind::UnclosedQuote);
        assert_eq!(error.cursor(), 1);
    }

    #[test]
    fn utf16_cursor_matches_java_string_index() {
        let mut reader = CommandReader::new("a🙂 b");
        reader.read();
        reader.read();

        assert_eq!(reader.cursor(), "a🙂".len());
        assert_eq!(reader.utf16_cursor(), 3);
    }
}
