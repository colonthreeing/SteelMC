//! Server command function registry.

use std::{collections::HashMap, error::Error, fmt, sync::Arc};

use steel_utils::Identifier;

use crate::command::graph::CommandFunctionArgumentValue;

const MAX_COMMAND_FUNCTION_LINE_LENGTH: usize = 2_000_000;

/// One registered command function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandFunction {
    id: Identifier,
    commands: Arc<[String]>,
}

impl CommandFunction {
    /// Creates a command function from parsed command lines.
    #[must_use]
    pub fn new(id: Identifier, commands: impl Into<Arc<[String]>>) -> Self {
        Self {
            id,
            commands: commands.into(),
        }
    }

    /// Returns this function's identifier.
    #[must_use]
    pub const fn id(&self) -> &Identifier {
        &self.id
    }

    /// Returns command lines in execution order.
    #[must_use]
    pub fn commands(&self) -> &[String] {
        &self.commands
    }

    /// Parses `.mcfunction` source into normalized command lines.
    ///
    /// This performs vanilla source-level handling: trim each line, skip blank
    /// lines and comments, join trailing-backslash continuations, reject command
    /// lines prefixed with `/`, and enforce vanilla's command line length
    /// limit. Command graph validation is performed by the provider that loads
    /// functions into the live dispatcher.
    ///
    /// # Errors
    ///
    /// Returns a source parse error when the function file uses syntax Steel
    /// cannot faithfully load yet.
    pub fn from_source(id: Identifier, source: &str) -> Result<Self, CommandFunctionParseError> {
        Ok(Self::new(id, parse_function_source(source)?))
    }
}

/// Error returned when parsing `.mcfunction` source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandFunctionParseError {
    line: usize,
    kind: CommandFunctionParseErrorKind,
}

impl CommandFunctionParseError {
    fn new(line: usize, kind: CommandFunctionParseErrorKind) -> Self {
        Self { line, kind }
    }

    /// Returns the 1-based source line number where parsing failed.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }

    /// Returns the specific parse error.
    #[must_use]
    pub const fn kind(&self) -> &CommandFunctionParseErrorKind {
        &self.kind
    }
}

impl fmt::Display for CommandFunctionParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "command function parse error on line {}: ", self.line)?;
        match &self.kind {
            CommandFunctionParseErrorKind::LineContinuationAtEnd => {
                f.write_str("line continuation at end of file")
            }
            CommandFunctionParseErrorKind::CommandTooLong { length, max } => {
                write!(
                    f,
                    "command is too long: {length} UTF-16 code units, maximum is {max}"
                )
            }
            CommandFunctionParseErrorKind::LeadingDoubleSlashComment => {
                f.write_str("unknown or invalid command; use '#' instead of '//' for comments")
            }
            CommandFunctionParseErrorKind::LeadingSlash { command } => write!(
                f,
                "unknown or invalid command; did you mean '{command}'? Do not use a preceding slash"
            ),
            CommandFunctionParseErrorKind::MacrosUnsupported => {
                f.write_str("command function macros need macro execution support")
            }
        }
    }
}

impl Error for CommandFunctionParseError {}

/// Specific `.mcfunction` source parse error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandFunctionParseErrorKind {
    /// A trailing `\` requested another physical line, but the file ended.
    LineContinuationAtEnd,
    /// A command exceeded vanilla's maximum source line length.
    CommandTooLong {
        /// Parsed UTF-16 code unit length.
        length: usize,
        /// Maximum accepted UTF-16 code unit length.
        max: usize,
    },
    /// The line starts with `//`, which vanilla rejects as an invalid command.
    LeadingDoubleSlashComment,
    /// The line starts with `/`, which is not valid inside functions.
    LeadingSlash {
        /// Command name after the slash, used for the vanilla-style hint.
        command: String,
    },
    /// The line starts with `$`, but Steel has no command macro runtime yet.
    MacrosUnsupported,
}

/// Server-level command function registry.
#[derive(Clone, Debug, Default)]
pub struct CommandFunctionRegistry {
    functions: HashMap<Identifier, CommandFunction>,
    tags: HashMap<Identifier, Vec<Identifier>>,
}

impl CommandFunctionRegistry {
    /// Registers or replaces one command function.
    pub fn insert_function(&mut self, function: CommandFunction) {
        self.functions.insert(function.id.clone(), function);
    }

    /// Registers or replaces a command function tag.
    pub fn insert_tag(&mut self, id: Identifier, functions: Vec<Identifier>) {
        self.tags.insert(id, functions);
    }

    /// Returns one command function by ID.
    #[must_use]
    pub fn function(&self, id: &Identifier) -> Option<CommandFunction> {
        self.functions.get(id).cloned()
    }

    /// Returns functions referenced by a tag, or an empty list for an unknown tag.
    #[must_use]
    pub fn tag(&self, id: &Identifier) -> Vec<CommandFunction> {
        self.tags.get(id).map_or_else(Vec::new, |functions| {
            functions
                .iter()
                .filter_map(|function| self.function(function))
                .collect()
        })
    }

    /// Resolves a parsed command function argument into concrete functions.
    #[must_use]
    pub fn resolve_argument(
        &self,
        argument: &CommandFunctionArgumentValue,
    ) -> Vec<CommandFunction> {
        match argument {
            CommandFunctionArgumentValue::Function(id) => self.function(id).into_iter().collect(),
            CommandFunctionArgumentValue::Tag(id) => self.tag(id),
        }
    }

    /// Returns registered function IDs.
    pub fn function_ids(&self) -> Vec<&Identifier> {
        sorted_ids(self.functions.keys())
    }

    /// Returns registered tag IDs.
    pub fn tag_ids(&self) -> Vec<&Identifier> {
        sorted_ids(self.tags.keys())
    }
}

fn sorted_ids<'a>(ids: impl Iterator<Item = &'a Identifier>) -> Vec<&'a Identifier> {
    let mut ids = ids.collect::<Vec<_>>();
    ids.sort_by_cached_key(|id| id.to_string());
    ids
}

fn parse_function_source(source: &str) -> Result<Vec<String>, CommandFunctionParseError> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut commands = Vec::new();
    let mut line_index = 0;
    while line_index < lines.len() {
        let line_number = line_index + 1;
        let mut line = lines[line_index].trim().to_owned();
        line_index += 1;

        if should_concatenate_next_line(&line) {
            loop {
                let Some(next_line) = lines.get(line_index) else {
                    return Err(CommandFunctionParseError::new(
                        line_number,
                        CommandFunctionParseErrorKind::LineContinuationAtEnd,
                    ));
                };
                line.pop();
                line.push_str(next_line.trim());
                check_command_line_length(&line, line_number)?;
                line_index += 1;
                if !should_concatenate_next_line(&line) {
                    break;
                }
            }
        } else {
            check_command_line_length(&line, line_number)?;
        }

        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        validate_command_function_line(&line, line_number)?;
        commands.push(line);
    }

    Ok(commands)
}

fn should_concatenate_next_line(line: &str) -> bool {
    line.ends_with('\\')
}

fn check_command_line_length(
    line: &str,
    line_number: usize,
) -> Result<(), CommandFunctionParseError> {
    let length = line.encode_utf16().count();
    if length > MAX_COMMAND_FUNCTION_LINE_LENGTH {
        return Err(CommandFunctionParseError::new(
            line_number,
            CommandFunctionParseErrorKind::CommandTooLong {
                length,
                max: MAX_COMMAND_FUNCTION_LINE_LENGTH,
            },
        ));
    }
    Ok(())
}

fn validate_command_function_line(
    line: &str,
    line_number: usize,
) -> Result<(), CommandFunctionParseError> {
    if line.starts_with("//") {
        return Err(CommandFunctionParseError::new(
            line_number,
            CommandFunctionParseErrorKind::LeadingDoubleSlashComment,
        ));
    }
    if let Some(command) = line.strip_prefix('/') {
        return Err(CommandFunctionParseError::new(
            line_number,
            CommandFunctionParseErrorKind::LeadingSlash {
                command: read_unquoted_command_name(command),
            },
        ));
    }
    if line.starts_with('$') {
        return Err(CommandFunctionParseError::new(
            line_number,
            CommandFunctionParseErrorKind::MacrosUnsupported,
        ));
    }
    Ok(())
}

fn read_unquoted_command_name(input: &str) -> String {
    input
        .chars()
        .take_while(|ch| is_brigadier_unquoted_char(*ch))
        .collect()
}

fn is_brigadier_unquoted_char(ch: char) -> bool {
    matches!(ch, '0'..='9' | 'A'..='Z' | 'a'..='z' | '_' | '-' | '.' | '+')
}

#[cfg(test)]
mod tests {
    use steel_utils::Identifier;

    use crate::command::graph::CommandFunctionArgumentValue;

    use super::{
        CommandFunction, CommandFunctionParseErrorKind, CommandFunctionRegistry,
        MAX_COMMAND_FUNCTION_LINE_LENGTH,
    };

    #[test]
    fn registry_resolves_functions_and_tags() {
        let first = Identifier::new_static("test", "first");
        let second = Identifier::new_static("test", "second");
        let tag = Identifier::new_static("test", "load");
        let mut registry = CommandFunctionRegistry::default();

        registry.insert_function(CommandFunction::new(
            first.clone(),
            vec!["say one".to_owned()],
        ));
        registry.insert_function(CommandFunction::new(
            second.clone(),
            vec!["say two".to_owned()],
        ));
        registry.insert_tag(tag.clone(), vec![second.clone(), first.clone()]);

        assert_eq!(
            registry.function(&first).map(|function| function.id),
            Some(first.clone())
        );
        assert_eq!(
            registry
                .tag(&tag)
                .into_iter()
                .map(|function| function.id)
                .collect::<Vec<_>>(),
            vec![second, Identifier::new_static("test", "first")]
        );
        assert_eq!(
            registry
                .resolve_argument(&CommandFunctionArgumentValue::Function(first.clone()))
                .into_iter()
                .map(|function| function.id)
                .collect::<Vec<_>>(),
            vec![first]
        );
        assert_eq!(
            registry
                .resolve_argument(&CommandFunctionArgumentValue::Tag(tag.clone()))
                .into_iter()
                .map(|function| function.id)
                .collect::<Vec<_>>(),
            vec![
                Identifier::new_static("test", "second"),
                Identifier::new_static("test", "first")
            ]
        );
        assert!(
            registry
                .tag(&Identifier::new_static("test", "missing"))
                .is_empty()
        );
    }

    #[test]
    fn function_source_parser_trims_comments_and_joins_continuations() {
        let function = CommandFunction::from_source(
            Identifier::new_static("test", "load"),
            "# comment\n say one \n\nsay \\\n two\n",
        )
        .expect("function source parses");

        assert_eq!(function.commands(), ["say one", "say two"]);
    }

    #[test]
    fn function_source_parser_rejects_line_continuation_at_end() {
        let error = CommandFunction::from_source(Identifier::new_static("test", "bad"), "say \\")
            .expect_err("unterminated continuation rejects");

        assert_eq!(error.line(), 1);
        assert!(matches!(
            error.kind(),
            CommandFunctionParseErrorKind::LineContinuationAtEnd
        ));
    }

    #[test]
    fn function_source_parser_rejects_leading_slash() {
        let error =
            CommandFunction::from_source(Identifier::new_static("test", "bad"), "/say/path hi")
                .expect_err("slash-prefixed command rejects");

        assert_eq!(error.line(), 1);
        assert!(matches!(
            error.kind(),
            CommandFunctionParseErrorKind::LeadingSlash { command } if command == "say"
        ));
    }

    #[test]
    fn function_source_parser_rejects_double_slash_comments() {
        let error =
            CommandFunction::from_source(Identifier::new_static("test", "bad"), "// comment")
                .expect_err("double-slash comment rejects");

        assert!(matches!(
            error.kind(),
            CommandFunctionParseErrorKind::LeadingDoubleSlashComment
        ));
    }

    #[test]
    fn function_source_parser_rejects_macros_until_runtime_exists() {
        let error =
            CommandFunction::from_source(Identifier::new_static("test", "bad"), "$say $(value)")
                .expect_err("macro line rejects");

        assert!(matches!(
            error.kind(),
            CommandFunctionParseErrorKind::MacrosUnsupported
        ));
    }

    #[test]
    fn function_source_parser_enforces_vanilla_line_length_limit() {
        let source = "a".repeat(MAX_COMMAND_FUNCTION_LINE_LENGTH + 1);
        let error = CommandFunction::from_source(Identifier::new_static("test", "bad"), &source)
            .expect_err("overlong command rejects");

        assert!(matches!(
            error.kind(),
            CommandFunctionParseErrorKind::CommandTooLong { length, max }
                if *length == MAX_COMMAND_FUNCTION_LINE_LENGTH + 1
                    && *max == MAX_COMMAND_FUNCTION_LINE_LENGTH
        ));
    }
}
