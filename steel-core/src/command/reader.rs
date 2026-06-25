//! Cursor-based command input reader.

use crate::command::graph::{CommandParseError, CommandParseErrorKind};

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

    /// Requires at least one whitespace character, then skips the full whitespace run.
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
                if self.peek() == Some('"') {
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

    fn read_unquoted_string(&mut self) -> Result<String, CommandParseError> {
        let start = self.cursor;
        while self.peek().is_some_and(|ch| !ch.is_whitespace()) {
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
        let quote_cursor = self.absolute_cursor();
        debug_assert_eq!(self.read(), Some('"'));

        let mut value = String::new();
        while let Some(ch) = self.read() {
            match ch {
                '"' => return Ok(value),
                '\\' => value.push(self.read_escaped_char()?),
                _ => value.push(ch),
            }
        }

        Err(CommandParseError::new(
            CommandParseErrorKind::UnclosedQuote,
            quote_cursor,
        ))
    }

    fn read_escaped_char(&mut self) -> Result<char, CommandParseError> {
        let escape_cursor = self.absolute_cursor();
        match self.read() {
            Some(ch @ ('\\' | '"')) => Ok(ch),
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
