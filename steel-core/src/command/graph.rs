//! Dynamic command graph, parsers, and structured parse results.

use std::sync::Arc;

use steel_protocol::packets::game::{
    ArgumentStringTypeBehavior, ArgumentType, CommandNode as ProtocolCommandNode, CommandNodeInfo,
    SuggestionEntry, SuggestionType,
};

use crate::command::{
    context::CommandContext,
    error::CommandError,
    reader::{CommandReader, StringMode},
    requirement::{CommandInputContext, Requirement, RequirementContext},
};

/// Structured command parse error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandParseError {
    kind: CommandParseErrorKind,
    cursor: usize,
}

impl CommandParseError {
    /// Creates a parse error at `cursor`.
    #[must_use]
    pub const fn new(kind: CommandParseErrorKind, cursor: usize) -> Self {
        Self { kind, cursor }
    }

    /// Returns the error kind.
    #[must_use]
    pub const fn kind(&self) -> &CommandParseErrorKind {
        &self.kind
    }

    /// Returns the byte cursor in the original command input.
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    fn is_better_than(&self, other: &Self) -> bool {
        self.cursor > other.cursor
            || self.cursor == other.cursor && self.kind.precedence() > other.kind.precedence()
    }
}

/// Specific command parse error kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandParseErrorKind {
    /// The input contained no command.
    EmptyCommand,
    /// The parser expected whitespace between command nodes.
    ExpectedWhitespace,
    /// The parser expected another argument.
    ExpectedArgument,
    /// The parser expected a literal.
    ExpectedLiteral(String),
    /// The command is unknown at this cursor.
    UnknownCommand,
    /// The command path is valid but has no executable at this cursor.
    IncompleteCommand,
    /// Extra input remained after an executable command path.
    TrailingData,
    /// A quoted string was not closed.
    UnclosedQuote,
    /// A quoted string used an invalid escape.
    InvalidEscape(char),
    /// A boolean argument was invalid.
    InvalidBool(String),
    /// An integer argument was invalid.
    InvalidInteger(String),
    /// An integer argument was below its minimum.
    IntegerTooLow {
        /// Parsed value.
        value: i32,
        /// Minimum accepted value.
        min: i32,
    },
    /// An integer argument was above its maximum.
    IntegerTooHigh {
        /// Parsed value.
        value: i32,
        /// Maximum accepted value.
        max: i32,
    },
}

impl CommandParseErrorKind {
    const fn precedence(&self) -> u8 {
        match self {
            Self::TrailingData => 7,
            Self::InvalidBool(_)
            | Self::InvalidInteger(_)
            | Self::IntegerTooLow { .. }
            | Self::IntegerTooHigh { .. }
            | Self::UnclosedQuote
            | Self::InvalidEscape(_) => 6,
            Self::ExpectedArgument => 5,
            Self::IncompleteCommand => 4,
            Self::ExpectedWhitespace => 3,
            Self::ExpectedLiteral(_) => 2,
            Self::UnknownCommand => 1,
            Self::EmptyCommand => 0,
        }
    }
}

/// A parsed command argument value.
#[derive(Clone, Debug, PartialEq)]
pub enum ParsedArgument {
    /// Boolean argument.
    Bool(bool),
    /// 32-bit signed integer argument.
    I32(i32),
    /// String-like argument.
    String(String),
}

/// Typed command argument storage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedArguments {
    values: Vec<ParsedArgumentEntry>,
}

#[derive(Clone, Debug, PartialEq)]
struct ParsedArgumentEntry {
    name: String,
    value: ParsedArgument,
}

impl ParsedArguments {
    /// Stores an argument value.
    pub fn insert(&mut self, name: impl Into<String>, value: ParsedArgument) {
        let name = name.into();
        if let Some(existing) = self.values.iter_mut().find(|entry| entry.name == name) {
            existing.value = value;
            return;
        }

        self.values.push(ParsedArgumentEntry { name, value });
    }

    /// Returns a typed argument by name.
    ///
    /// # Errors
    ///
    /// Returns an error when the argument is missing or has the wrong type.
    pub fn get<T: FromParsedArgument>(&self, name: &str) -> Result<T, ParsedArgumentError> {
        let value = self
            .values
            .iter()
            .rev()
            .find_map(|entry| (entry.name == name).then_some(&entry.value))
            .ok_or_else(|| ParsedArgumentError::Missing(name.to_owned()))?;

        T::from_parsed_argument(value).ok_or_else(|| ParsedArgumentError::WrongType {
            name: name.to_owned(),
            expected: T::TYPE_NAME,
            actual: value.type_name(),
        })
    }
}

impl ParsedArgument {
    const fn type_name(&self) -> &'static str {
        match self {
            Self::Bool(_) => "bool",
            Self::I32(_) => "i32",
            Self::String(_) => "string",
        }
    }
}

/// Conversion from a parsed argument.
pub trait FromParsedArgument: Sized {
    /// Expected type name for diagnostics.
    const TYPE_NAME: &'static str;

    /// Converts from a parsed argument if the type matches.
    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self>;
}

impl FromParsedArgument for bool {
    const TYPE_NAME: &'static str = "bool";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Bool(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for i32 {
    const TYPE_NAME: &'static str = "i32";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::I32(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for String {
    const TYPE_NAME: &'static str = "string";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::String(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

/// Error returned when reading a typed parsed argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParsedArgumentError {
    /// The requested argument was not parsed.
    Missing(String),
    /// The requested argument has a different type.
    WrongType {
        /// Argument name.
        name: String,
        /// Expected type name.
        expected: &'static str,
        /// Actual type name.
        actual: &'static str,
    },
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

    /// Returns protocol parser metadata for this argument.
    fn usage(&self) -> (ArgumentType, Option<SuggestionType>);

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
}

/// Command execution result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandResult {
    /// Number of successful command results.
    pub success_count: i32,
}

impl CommandResult {
    /// Creates a successful command result.
    #[must_use]
    pub const fn success() -> Self {
        Self { success_count: 1 }
    }
}

/// Suggestions for a command input range.
#[derive(Clone, Debug)]
pub struct SuggestionResult {
    /// Suggested entries.
    pub suggestions: Vec<SuggestionEntry>,
    /// UTF-16 start position in the original command string.
    pub start: i32,
    /// UTF-16 length to replace.
    pub length: i32,
}

type CommandExecutor = Arc<
    dyn Fn(&mut CommandContext, &ParsedArguments) -> Result<CommandResult, CommandError>
        + Send
        + Sync,
>;

/// A successfully parsed command.
#[derive(Clone)]
pub struct ParseResults {
    input: String,
    arguments: ParsedArguments,
    path: Vec<String>,
    executor: CommandExecutor,
}

impl std::fmt::Debug for ParseResults {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParseResults")
            .field("input", &self.input)
            .field("arguments", &self.arguments)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl ParseResults {
    /// Returns the original input.
    #[must_use]
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Returns parsed arguments.
    #[must_use]
    pub const fn arguments(&self) -> &ParsedArguments {
        &self.arguments
    }

    /// Returns the matched command path.
    #[must_use]
    pub fn path(&self) -> &[String] {
        &self.path
    }

    /// Executes this parsed command.
    ///
    /// # Errors
    ///
    /// Returns a command execution error from the matched executor.
    pub fn execute(&self, context: &mut CommandContext) -> Result<CommandResult, CommandError> {
        (self.executor)(context, &self.arguments)
    }
}

/// Dynamic command graph.
#[derive(Default)]
pub struct CommandGraph {
    roots: Vec<CommandNode>,
}

impl CommandGraph {
    /// Creates an empty command graph.
    #[must_use]
    pub const fn new() -> Self {
        Self { roots: Vec::new() }
    }

    /// Adds a root command node.
    #[must_use]
    pub fn with_root(mut self, root: CommandNodeBuilder) -> Self {
        self.roots.push(root.build());
        self
    }

    /// Adds a root command node.
    pub fn register_root(&mut self, root: CommandNodeBuilder) {
        self.roots.push(root.build());
    }

    /// Returns true when this graph has a usable root literal named `name`.
    #[must_use]
    pub fn has_root(&self, name: &str, context: &dyn RequirementContext) -> bool {
        self.roots.iter().any(|root| {
            root.requirement.allows(context)
                && matches!(&root.kind, CommandNodeKind::Literal(root_name) if root_name == name)
        })
    }

    /// Appends usable graph nodes to a protocol command tree.
    pub fn usage(
        &self,
        buffer: &mut Vec<ProtocolCommandNode>,
        root_children: &mut Vec<i32>,
        context: &dyn RequirementContext,
    ) {
        for root in &self.roots {
            root.usage(buffer, root_children, context);
        }
    }

    /// Adds usable root literal suggestions matching `prefix`.
    pub fn add_root_suggestions(
        &self,
        prefix: &str,
        suggestions: &mut Vec<SuggestionEntry>,
        context: &dyn RequirementContext,
    ) {
        for root in &self.roots {
            if !root.requirement.allows(context) {
                continue;
            }

            let CommandNodeKind::Literal(name) = &root.kind else {
                continue;
            };

            if name.starts_with(prefix) {
                suggestions.push(SuggestionEntry::new(name.clone()));
            }
        }
    }

    /// Returns suggestions for `input` using the same graph and requirements as parsing.
    #[must_use]
    pub fn suggest(
        &self,
        input: &str,
        context: &dyn CommandInputContext,
    ) -> Option<SuggestionResult> {
        let (input_without_slash, cursor_offset) = input
            .strip_prefix('/')
            .map_or((input, 0), |stripped| (stripped, 1));
        let mut reader = CommandReader::with_offset(input_without_slash, cursor_offset);
        reader.skip_whitespace();

        suggest_children(&reader, &self.roots, ParsedArguments::default(), context)
    }

    /// Parses `input` for `context`.
    ///
    /// # Errors
    ///
    /// Returns a structured parse error when the input does not resolve to an
    /// executable command.
    pub fn parse(
        &self,
        input: &str,
        context: &dyn CommandInputContext,
    ) -> Result<ParseResults, CommandParseError> {
        let (input_without_slash, cursor_offset) = input
            .strip_prefix('/')
            .map_or((input, 0), |stripped| (stripped, 1));
        let mut reader = CommandReader::with_offset(input_without_slash, cursor_offset);
        reader.skip_whitespace();

        if !reader.can_read() {
            return Err(CommandParseError::new(
                CommandParseErrorKind::EmptyCommand,
                reader.absolute_cursor(),
            ));
        }

        parse_children(
            input,
            &mut reader,
            &self.roots,
            ParsedArguments::default(),
            Vec::new(),
            context,
        )
    }
}

fn parse_children(
    input: &str,
    reader: &mut CommandReader<'_>,
    children: &[CommandNode],
    arguments: ParsedArguments,
    path: Vec<String>,
    context: &dyn CommandInputContext,
) -> Result<ParseResults, CommandParseError> {
    let mut best_error = None;
    let mut usable_child_seen = false;

    for child in children {
        if !child.requirement.allows(context) {
            continue;
        }

        usable_child_seen = true;
        let mut child_reader = reader.clone();
        let mut child_arguments = arguments.clone();
        let mut child_path = path.clone();

        match child.parse_self(&mut child_reader, &mut child_arguments, context) {
            Ok(()) => {
                child_path.push(child.display_name().to_owned());
                match parse_after_node(
                    input,
                    child,
                    &mut child_reader,
                    child_arguments,
                    child_path,
                    context,
                ) {
                    Ok(result) => return Ok(result),
                    Err(error) => keep_best_error(&mut best_error, error),
                }
            }
            Err(error) => keep_best_error(&mut best_error, error),
        }
    }

    Err(best_error.unwrap_or_else(|| {
        CommandParseError::new(
            if usable_child_seen {
                CommandParseErrorKind::ExpectedArgument
            } else {
                CommandParseErrorKind::UnknownCommand
            },
            reader.absolute_cursor(),
        )
    }))
}

fn parse_after_node(
    input: &str,
    node: &CommandNode,
    reader: &mut CommandReader<'_>,
    arguments: ParsedArguments,
    path: Vec<String>,
    context: &dyn CommandInputContext,
) -> Result<ParseResults, CommandParseError> {
    if !reader.can_read() {
        return node.executable(input, arguments, path).ok_or_else(|| {
            CommandParseError::new(
                CommandParseErrorKind::IncompleteCommand,
                reader.absolute_cursor(),
            )
        });
    }

    if !reader.peek().is_some_and(char::is_whitespace) {
        return Err(CommandParseError::new(
            CommandParseErrorKind::TrailingData,
            reader.absolute_cursor(),
        ));
    }

    reader.expect_whitespace()?;
    if !reader.can_read() {
        return node.executable(input, arguments, path).ok_or_else(|| {
            CommandParseError::new(
                CommandParseErrorKind::IncompleteCommand,
                reader.absolute_cursor(),
            )
        });
    }

    if node.children.is_empty() {
        return Err(CommandParseError::new(
            CommandParseErrorKind::TrailingData,
            reader.absolute_cursor(),
        ));
    }

    parse_children(input, reader, &node.children, arguments, path, context)
}

fn keep_best_error(best_error: &mut Option<CommandParseError>, error: CommandParseError) {
    if best_error
        .as_ref()
        .is_none_or(|best| error.is_better_than(best))
    {
        *best_error = Some(error);
    }
}

fn suggest_children(
    reader: &CommandReader<'_>,
    children: &[CommandNode],
    arguments: ParsedArguments,
    context: &dyn CommandInputContext,
) -> Option<SuggestionResult> {
    let mut best_result = None;

    for child in children {
        if !child.requirement.allows(context) {
            continue;
        }

        if let Some(result) = child.suggest(reader, arguments.clone(), context) {
            keep_best_suggestion(&mut best_result, result);
        }
    }

    best_result
}

fn keep_best_suggestion(best_result: &mut Option<SuggestionResult>, result: SuggestionResult) {
    let Some(best) = best_result else {
        *best_result = Some(result);
        return;
    };

    if result.start > best.start {
        *best = result;
        return;
    }

    if result.start == best.start && result.length == best.length {
        best.suggestions.extend(result.suggestions);
    }
}

fn make_suggestion_result(
    reader: &CommandReader<'_>,
    suggestions: Vec<SuggestionEntry>,
) -> Option<SuggestionResult> {
    if suggestions.is_empty() {
        return None;
    }

    let token = suggestion_token(reader);
    Some(SuggestionResult {
        suggestions,
        start: token.start,
        length: token.length,
    })
}

fn suggestion_token(reader: &CommandReader<'_>) -> SuggestionToken {
    let remaining = reader.remaining();
    let prefix = remaining
        .chars()
        .take_while(|ch| !ch.is_whitespace())
        .collect::<String>();
    let length = prefix.encode_utf16().count() as i32;
    let is_at_end = remaining[prefix.len()..].is_empty();

    SuggestionToken {
        prefix,
        start: reader.absolute_utf16_cursor() as i32,
        length,
        is_at_end,
    }
}

struct SuggestionToken {
    prefix: String,
    start: i32,
    length: i32,
    is_at_end: bool,
}

/// Builds a command graph node.
pub struct CommandNodeBuilder {
    kind: CommandNodeKind,
    requirement: Requirement,
    children: Vec<CommandNodeBuilder>,
    executor: Option<CommandExecutor>,
}

impl CommandNodeBuilder {
    /// Adds a child node.
    #[must_use]
    pub fn then(mut self, child: Self) -> Self {
        self.children.push(child);
        self
    }

    /// Adds a requirement to this node.
    #[must_use]
    pub fn requires(mut self, requirement: Requirement) -> Self {
        self.requirement = self.requirement.and(requirement);
        self
    }

    /// Marks this node executable.
    #[must_use]
    pub fn executes(
        mut self,
        executor: impl Fn(&mut CommandContext, &ParsedArguments) -> Result<CommandResult, CommandError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.executor = Some(Arc::new(executor));
        self
    }

    fn build(self) -> CommandNode {
        CommandNode {
            kind: self.kind,
            requirement: self.requirement,
            children: self.children.into_iter().map(Self::build).collect(),
            executor: self.executor,
        }
    }
}

/// Creates a literal node builder.
#[must_use]
pub fn literal(name: impl Into<String>) -> CommandNodeBuilder {
    CommandNodeBuilder {
        kind: CommandNodeKind::Literal(name.into()),
        requirement: Requirement::Always,
        children: Vec::new(),
        executor: None,
    }
}

/// Creates an argument node builder.
#[must_use]
pub fn argument(
    name: impl Into<String>,
    parser: impl CommandArgumentParser + 'static,
) -> CommandNodeBuilder {
    CommandNodeBuilder {
        kind: CommandNodeKind::Argument {
            name: name.into(),
            parser: Arc::new(parser),
        },
        requirement: Requirement::Always,
        children: Vec::new(),
        executor: None,
    }
}

struct CommandNode {
    kind: CommandNodeKind,
    requirement: Requirement,
    children: Vec<CommandNode>,
    executor: Option<CommandExecutor>,
}

impl CommandNode {
    fn parse_self(
        &self,
        reader: &mut CommandReader<'_>,
        arguments: &mut ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Result<(), CommandParseError> {
        match &self.kind {
            CommandNodeKind::Literal(expected) => {
                let cursor = reader.absolute_cursor();
                let actual = reader.read_string(StringMode::SingleWord)?;
                if actual == *expected {
                    Ok(())
                } else {
                    Err(CommandParseError::new(
                        CommandParseErrorKind::ExpectedLiteral(expected.clone()),
                        cursor,
                    ))
                }
            }
            CommandNodeKind::Argument { name, parser } => {
                let value = parser.parse(reader, context)?;
                arguments.insert(name, value);
                Ok(())
            }
        }
    }

    fn display_name(&self) -> &str {
        match &self.kind {
            CommandNodeKind::Literal(name) | CommandNodeKind::Argument { name, .. } => name,
        }
    }

    fn executable(
        &self,
        input: &str,
        arguments: ParsedArguments,
        path: Vec<String>,
    ) -> Option<ParseResults> {
        Some(ParseResults {
            input: input.to_owned(),
            arguments,
            path,
            executor: Arc::clone(self.executor.as_ref()?),
        })
    }

    fn suggest(
        &self,
        reader: &CommandReader<'_>,
        mut arguments: ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Option<SuggestionResult> {
        let token = suggestion_token(reader);
        let direct_suggestions = match &self.kind {
            CommandNodeKind::Literal(name)
                if token.is_at_end && name.starts_with(&token.prefix) =>
            {
                make_suggestion_result(reader, vec![SuggestionEntry::new(name.clone())])
            }
            CommandNodeKind::Argument { parser, .. } if token.is_at_end => {
                make_suggestion_result(reader, parser.suggest(&token.prefix, &arguments, context))
            }
            CommandNodeKind::Literal(_) | CommandNodeKind::Argument { .. } => None,
        };

        let mut parsed_reader = reader.clone();
        if self
            .parse_self(&mut parsed_reader, &mut arguments, context)
            .is_err()
        {
            return direct_suggestions;
        }

        let mut result = direct_suggestions;
        if let Some(deeper) =
            self.suggest_after_successful_parse(&mut parsed_reader, arguments, context)
        {
            keep_best_suggestion(&mut result, deeper);
        }
        result
    }

    fn suggest_after_successful_parse(
        &self,
        reader: &mut CommandReader<'_>,
        arguments: ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Option<SuggestionResult> {
        if !reader.can_read() {
            return None;
        }

        if !reader.peek().is_some_and(char::is_whitespace) {
            return None;
        }

        reader.skip_whitespace();
        suggest_children(reader, &self.children, arguments, context)
    }

    fn usage(
        &self,
        buffer: &mut Vec<ProtocolCommandNode>,
        siblings: &mut Vec<i32>,
        context: &dyn RequirementContext,
    ) {
        if !self.requirement.allows(context) {
            return;
        }

        let node_index = buffer.len();
        buffer.push(ProtocolCommandNode::new_root());
        siblings.push(node_index as i32);

        let mut children = Vec::new();
        for child in &self.children {
            child.usage(buffer, &mut children, context);
        }

        let mut info = CommandNodeInfo::new(children);
        if self.executor.is_some() {
            info = info.chain(CommandNodeInfo::new_executable());
        }

        buffer[node_index] = match &self.kind {
            CommandNodeKind::Literal(name) => ProtocolCommandNode::new_literal(info, name.clone()),
            CommandNodeKind::Argument { name, parser } => {
                ProtocolCommandNode::new_argument(info, name.clone(), parser.usage())
            }
        };
    }
}

enum CommandNodeKind {
    Literal(String),
    Argument {
        name: String,
        parser: Arc<dyn CommandArgumentParser>,
    },
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use crate::command::{
        graph::{
            BoolParser, CommandGraph, CommandParseErrorKind, CommandResult, IntegerParser,
            StringParser, SuggestionResult, argument, literal,
        },
        reader::StringMode,
        requirement::{
            CommandInputContext, CommandSourceKind, PermissionExpr, PermissionKey, Requirement,
            RequirementContext,
        },
    };

    struct TestContext {
        source_kind: CommandSourceKind,
        permissions: Vec<PermissionKey>,
    }

    impl RequirementContext for TestContext {
        fn source_kind(&self) -> CommandSourceKind {
            self.source_kind
        }

        fn has_permission(&self, permission: &PermissionExpr) -> bool {
            match permission {
                PermissionExpr::Key(key) => self.permissions.iter().any(|perm| perm.matches(key)),
                PermissionExpr::All(children) => {
                    children.iter().all(|child| self.has_permission(child))
                }
                PermissionExpr::Any(children) => {
                    children.iter().any(|child| self.has_permission(child))
                }
            }
        }
    }

    impl CommandInputContext for TestContext {}

    fn player_context() -> TestContext {
        TestContext {
            source_kind: CommandSourceKind::Player,
            permissions: Vec::new(),
        }
    }

    fn suggestion_texts(result: &SuggestionResult) -> Vec<String> {
        result
            .suggestions
            .iter()
            .map(|suggestion| suggestion.text.clone())
            .collect()
    }

    #[test]
    fn parses_literal_and_integer_argument() {
        let graph = CommandGraph::new().with_root(
            literal("give").then(
                argument("count", IntegerParser::bounded(Some(1), Some(64)))
                    .executes(|_, _| Ok(CommandResult::success())),
            ),
        );

        let result = graph
            .parse("/give 12", &player_context())
            .expect("command parses");

        assert_eq!(result.path(), ["give", "count"]);
        assert_eq!(result.arguments().get::<i32>("count"), Ok(12));
    }

    #[test]
    fn parses_named_arguments_for_executors() {
        let graph =
            CommandGraph::new().with_root(literal("flag").then(
                argument("enabled", BoolParser).executes(|_, _| Ok(CommandResult::success())),
            ));

        let result = graph
            .parse("flag true", &player_context())
            .expect("command parses");

        assert_eq!(result.arguments().get::<bool>("enabled"), Ok(true));
    }

    #[test]
    fn quoted_string_argument_keeps_spaces() {
        let graph = CommandGraph::new().with_root(
            literal("say").then(
                argument("message", StringParser::new(StringMode::QuotablePhrase))
                    .executes(|_, _| Ok(CommandResult::success())),
            ),
        );

        let result = graph
            .parse("say \"hello world\"", &player_context())
            .expect("command parses");

        assert_eq!(
            result.arguments().get::<String>("message"),
            Ok("hello world".to_owned())
        );
    }

    #[test]
    fn trailing_data_is_distinct_from_incomplete_command() {
        let graph = CommandGraph::new()
            .with_root(literal("list").executes(|_, _| Ok(CommandResult::success())));

        let error = graph
            .parse("list extra", &player_context())
            .expect_err("extra input should fail");

        assert_eq!(error.kind(), &CommandParseErrorKind::TrailingData);
        assert_eq!(error.cursor(), 5);
    }

    #[test]
    fn branch_errors_prefer_farthest_cursor() {
        let graph = CommandGraph::new().with_root(
            literal("root")
                .then(literal("foo").executes(|_, _| Ok(CommandResult::success())))
                .then(
                    literal("bar").then(
                        argument("count", IntegerParser::new())
                            .executes(|_, _| Ok(CommandResult::success())),
                    ),
                ),
        );

        let error = graph
            .parse("root bar nope", &player_context())
            .expect_err("invalid integer should fail");

        assert_eq!(
            error.kind(),
            &CommandParseErrorKind::InvalidInteger("nope".to_owned())
        );
        assert_eq!(error.cursor(), 9);
    }

    #[test]
    fn requirements_hide_unusable_nodes() {
        let denied = Arc::new(AtomicBool::new(false));
        let denied_in_executor = Arc::clone(&denied);
        let graph = CommandGraph::new().with_root(
            literal("admin")
                .requires(Requirement::Permission(PermissionExpr::key(
                    PermissionKey::parse("steel.admin").expect("key parses"),
                )))
                .executes(move |_, _| {
                    denied_in_executor.store(true, Ordering::Relaxed);
                    Ok(CommandResult::success())
                }),
        );

        let error = graph
            .parse("admin", &player_context())
            .expect_err("node should be hidden");

        assert_eq!(error.kind(), &CommandParseErrorKind::UnknownCommand);
        assert!(!denied.load(Ordering::Relaxed));
    }

    #[test]
    fn root_suggestions_include_partial_literals() {
        let graph = CommandGraph::new().with_root(literal("list"));

        let result = graph
            .suggest("li", &player_context())
            .expect("root suggestion");

        assert_eq!(suggestion_texts(&result), vec!["list".to_owned()]);
        assert_eq!(result.start, 0);
        assert_eq!(result.length, 2);
    }

    #[test]
    fn slash_root_suggestions_keep_client_offset() {
        let graph = CommandGraph::new().with_root(literal("list"));

        let result = graph
            .suggest("/li", &player_context())
            .expect("root suggestion");

        assert_eq!(suggestion_texts(&result), vec!["list".to_owned()]);
        assert_eq!(result.start, 1);
        assert_eq!(result.length, 2);
    }

    #[test]
    fn child_literal_suggestions_use_child_range() {
        let graph = CommandGraph::new().with_root(literal("list").then(literal("uuids")));

        let result = graph
            .suggest("list u", &player_context())
            .expect("child suggestion");

        assert_eq!(suggestion_texts(&result), vec!["uuids".to_owned()]);
        assert_eq!(result.start, 5);
        assert_eq!(result.length, 1);
    }

    #[test]
    fn trailing_space_suggests_children() {
        let graph = CommandGraph::new().with_root(literal("list").then(literal("uuids")));

        let result = graph
            .suggest("list ", &player_context())
            .expect("child suggestion");

        assert_eq!(suggestion_texts(&result), vec!["uuids".to_owned()]);
        assert_eq!(result.start, 5);
        assert_eq!(result.length, 0);
    }

    #[test]
    fn trailing_space_after_leaf_has_no_stale_suggestion() {
        let graph = CommandGraph::new().with_root(literal("list").then(literal("uuids")));

        assert!(graph.suggest("list uuids ", &player_context()).is_none());
    }

    #[test]
    fn bool_parser_suggests_values() {
        let graph =
            CommandGraph::new().with_root(literal("flag").then(argument("enabled", BoolParser)));

        let result = graph
            .suggest("flag f", &player_context())
            .expect("bool suggestion");

        assert_eq!(suggestion_texts(&result), vec!["false".to_owned()]);
        assert_eq!(result.start, 5);
        assert_eq!(result.length, 1);
    }
}
