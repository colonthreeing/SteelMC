//! Server command function registry.

use std::{
    collections::{HashMap, VecDeque},
    error::Error,
    fmt,
    sync::Arc,
};

use simdnbt::owned::{NbtCompound, NbtList, NbtTag};
use steel_utils::{Identifier, locks::SyncMutex};

use crate::command::{
    CommandDispatcher,
    graph::{CommandFunctionArgumentValue, CommandParseError},
    requirement::CommandInputContext,
};

const MAX_COMMAND_FUNCTION_LINE_LENGTH: usize = 2_000_000;
const MAX_MACRO_CACHE_ENTRIES: usize = 8;

/// One registered command function.
#[derive(Clone, Debug)]
pub struct CommandFunction {
    id: Identifier,
    commands: Arc<[String]>,
    entries: Arc<[CommandFunctionEntry]>,
    source_lines: Arc<[usize]>,
    macro_parameters: Arc<[String]>,
    macro_cache: Arc<SyncMutex<CommandFunctionMacroCache>>,
}

impl PartialEq for CommandFunction {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.commands == other.commands
            && self.entries == other.entries
            && self.source_lines == other.source_lines
            && self.macro_parameters == other.macro_parameters
    }
}

impl Eq for CommandFunction {}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CommandFunctionEntry {
    Plain {
        command_index: usize,
    },
    Macro {
        template: MacroTemplate,
        parameter_indices: Arc<[usize]>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MacroTemplate {
    segments: Arc<[String]>,
    variables: Arc<[String]>,
}

#[derive(Clone, Debug, Default)]
struct CommandFunctionMacroCache {
    entries: VecDeque<CommandFunctionMacroCacheEntry>,
}

#[derive(Clone, Debug)]
struct CommandFunctionMacroCacheEntry {
    parameter_values: Arc<[String]>,
    commands: Arc<[String]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InstantiatedCommandFunction {
    commands: Arc<[String]>,
}

impl InstantiatedCommandFunction {
    #[must_use]
    pub(crate) fn commands(&self) -> &[String] {
        &self.commands
    }
}

impl CommandFunction {
    /// Creates a command function from parsed command lines.
    #[must_use]
    pub fn new(id: Identifier, commands: impl Into<Arc<[String]>>) -> Self {
        let commands = commands.into();
        let source_lines = (1..=commands.len()).collect::<Vec<_>>().into();
        Self::new_with_source_lines(id, commands, source_lines)
    }

    fn new_with_source_lines(
        id: Identifier,
        commands: Arc<[String]>,
        source_lines: Arc<[usize]>,
    ) -> Self {
        debug_assert_eq!(commands.len(), source_lines.len());
        let entries = (0..commands.len())
            .map(|command_index| CommandFunctionEntry::Plain { command_index })
            .collect::<Vec<_>>()
            .into();
        Self::new_with_entries(id, commands, entries, source_lines, Vec::new().into())
    }

    fn new_with_entries(
        id: Identifier,
        commands: Arc<[String]>,
        entries: Arc<[CommandFunctionEntry]>,
        source_lines: Arc<[usize]>,
        macro_parameters: Arc<[String]>,
    ) -> Self {
        debug_assert_eq!(commands.len(), source_lines.len());
        debug_assert_eq!(commands.len(), entries.len());
        Self {
            id,
            commands,
            entries,
            source_lines,
            macro_parameters,
            macro_cache: Arc::new(SyncMutex::new(CommandFunctionMacroCache::default())),
        }
    }

    /// Returns this function's identifier.
    #[must_use]
    pub const fn id(&self) -> &Identifier {
        &self.id
    }

    /// Returns normalized source lines in execution order.
    ///
    /// Macro functions keep their source macro lines here, including the
    /// leading `$`; use the function instantiation path when executable
    /// command lines are needed.
    #[must_use]
    pub fn commands(&self) -> &[String] {
        &self.commands
    }

    /// Returns true when this function has at least one macro entry.
    #[must_use]
    pub fn is_macro(&self) -> bool {
        !self.macro_parameters.is_empty()
    }

    /// Returns the original source line for a parsed command.
    #[must_use]
    pub fn command_source_line(&self, command_index: usize) -> Option<usize> {
        self.source_lines.get(command_index).copied()
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
        let parsed = parse_function_source(source)?;
        Ok(Self::new_with_entries(
            id,
            parsed.commands.into(),
            parsed.entries.into(),
            parsed.source_lines.into(),
            parsed.macro_parameters.into(),
        ))
    }

    /// Validates each command line against the live command dispatcher.
    ///
    /// This mirrors vanilla's function load-time command parsing without
    /// deciding how the function will execute later.
    ///
    /// # Errors
    ///
    /// Returns the first command line that does not parse for `context`.
    pub fn validate_commands(
        &self,
        dispatcher: &CommandDispatcher,
        context: &dyn CommandInputContext,
    ) -> Result<(), CommandFunctionValidationError> {
        for (index, entry) in self.entries.iter().enumerate() {
            let CommandFunctionEntry::Plain { command_index } = entry else {
                continue;
            };
            let command = &self.commands[*command_index];
            if let Err(source) = dispatcher.graph.parse(command, context) {
                return Err(CommandFunctionValidationError::new(
                    self.command_source_line(index).unwrap_or(index + 1),
                    command.to_owned(),
                    source,
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn instantiate(
        &self,
        arguments: Option<&NbtCompound>,
        dispatcher: &CommandDispatcher,
        context: &dyn CommandInputContext,
    ) -> Result<InstantiatedCommandFunction, CommandFunctionInstantiationError> {
        if !self.is_macro() {
            return Ok(InstantiatedCommandFunction {
                commands: Arc::clone(&self.commands),
            });
        }

        let Some(arguments) = arguments else {
            return Err(CommandFunctionInstantiationError::missing_arguments(
                self.id.clone(),
            ));
        };
        let parameter_values = self.parameter_values(arguments)?;
        if let Some(commands) = self.cached_instantiation(&parameter_values) {
            return Ok(InstantiatedCommandFunction { commands });
        }

        let commands = self.substitute_and_parse(&parameter_values, dispatcher, context)?;
        self.insert_cached_instantiation(parameter_values.into(), Arc::clone(&commands));
        Ok(InstantiatedCommandFunction { commands })
    }

    fn parameter_values(
        &self,
        arguments: &NbtCompound,
    ) -> Result<Vec<String>, CommandFunctionInstantiationError> {
        self.macro_parameters
            .iter()
            .map(|parameter| {
                let Some(value) = arguments.get(parameter) else {
                    return Err(CommandFunctionInstantiationError::missing_argument(
                        self.id.clone(),
                        parameter.to_owned(),
                    ));
                };
                Ok(stringify_macro_argument(value))
            })
            .collect()
    }

    fn cached_instantiation(&self, parameter_values: &[String]) -> Option<Arc<[String]>> {
        let mut cache = self.macro_cache.lock();
        let index = cache
            .entries
            .iter()
            .position(|entry| entry.parameter_values.as_ref() == parameter_values)?;
        let entry = cache.entries.remove(index)?;
        let commands = Arc::clone(&entry.commands);
        cache.entries.push_back(entry);
        Some(commands)
    }

    fn insert_cached_instantiation(
        &self,
        parameter_values: Arc<[String]>,
        commands: Arc<[String]>,
    ) {
        let mut cache = self.macro_cache.lock();
        if cache.entries.len() >= MAX_MACRO_CACHE_ENTRIES {
            cache.entries.pop_front();
        }
        cache.entries.push_back(CommandFunctionMacroCacheEntry {
            parameter_values,
            commands,
        });
    }

    fn substitute_and_parse(
        &self,
        parameter_values: &[String],
        dispatcher: &CommandDispatcher,
        context: &dyn CommandInputContext,
    ) -> Result<Arc<[String]>, CommandFunctionInstantiationError> {
        let mut commands = Vec::with_capacity(self.entries.len());
        for (entry_index, entry) in self.entries.iter().enumerate() {
            let command = match entry {
                CommandFunctionEntry::Plain { command_index } => {
                    self.commands[*command_index].to_owned()
                }
                CommandFunctionEntry::Macro {
                    template,
                    parameter_indices,
                } => template.substitute(
                    parameter_indices
                        .iter()
                        .map(|index| parameter_values[*index].as_str()),
                )?,
            };

            if let Err(source) = dispatcher.graph.parse(&command, context) {
                return Err(CommandFunctionInstantiationError::invalid_command(
                    self.id.clone(),
                    self.command_source_line(entry_index)
                        .unwrap_or(entry_index + 1),
                    command,
                    source,
                ));
            }
            commands.push(command);
        }
        Ok(commands.into())
    }
}

/// Error returned when a parsed command function does not validate.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandFunctionValidationError {
    line: usize,
    command: String,
    source: CommandParseError,
}

impl CommandFunctionValidationError {
    fn new(line: usize, command: String, source: CommandParseError) -> Self {
        Self {
            line,
            command,
            source,
        }
    }

    /// Returns the 1-based source line where validation failed.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }

    /// Returns the command line that failed validation.
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }

    /// Returns the underlying command parse error.
    #[must_use]
    pub const fn source(&self) -> &CommandParseError {
        &self.source
    }
}

impl fmt::Display for CommandFunctionValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "command function validation error on line {} while parsing '{}': {:?}",
            self.line,
            self.command,
            self.source.kind()
        )
    }
}

impl Error for CommandFunctionValidationError {}

/// Error returned when a macro command function cannot instantiate.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandFunctionInstantiationError {
    kind: CommandFunctionInstantiationErrorKind,
}

impl CommandFunctionInstantiationError {
    fn missing_arguments(function: Identifier) -> Self {
        Self {
            kind: CommandFunctionInstantiationErrorKind::MissingArguments { function },
        }
    }

    fn missing_argument(function: Identifier, argument: String) -> Self {
        Self {
            kind: CommandFunctionInstantiationErrorKind::MissingArgument { function, argument },
        }
    }

    fn command_too_long(length: usize, max: usize) -> Self {
        Self {
            kind: CommandFunctionInstantiationErrorKind::CommandTooLong { length, max },
        }
    }

    fn invalid_command(
        function: Identifier,
        line: usize,
        command: String,
        source: CommandParseError,
    ) -> Self {
        Self {
            kind: CommandFunctionInstantiationErrorKind::InvalidCommand {
                function,
                line,
                command,
                source,
            },
        }
    }

    /// Returns the specific instantiation failure.
    #[must_use]
    pub const fn kind(&self) -> &CommandFunctionInstantiationErrorKind {
        &self.kind
    }
}

impl fmt::Display for CommandFunctionInstantiationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            CommandFunctionInstantiationErrorKind::MissingArguments { function } => {
                write!(f, "function {function} requires macro arguments")
            }
            CommandFunctionInstantiationErrorKind::MissingArgument { function, argument } => {
                write!(f, "function {function} missing macro argument '{argument}'")
            }
            CommandFunctionInstantiationErrorKind::CommandTooLong { length, max } => {
                write!(
                    f,
                    "instantiated macro command is too long: {length} UTF-16 code units, maximum is {max}"
                )
            }
            CommandFunctionInstantiationErrorKind::InvalidCommand {
                function,
                line,
                command,
                source,
            } => write!(
                f,
                "function {function} macro command on line {line} parsed as invalid command '{command}': {:?}",
                source.kind()
            ),
        }
    }
}

impl Error for CommandFunctionInstantiationError {}

/// Specific command function instantiation failure.
#[derive(Clone, Debug, PartialEq)]
pub enum CommandFunctionInstantiationErrorKind {
    /// A macro function was called without any argument compound.
    MissingArguments {
        /// Function identifier.
        function: Identifier,
    },
    /// A macro function argument compound did not contain one referenced key.
    MissingArgument {
        /// Function identifier.
        function: Identifier,
        /// Missing macro argument key.
        argument: String,
    },
    /// A substituted macro command exceeded vanilla's line length limit.
    CommandTooLong {
        /// Instantiated UTF-16 code unit length.
        length: usize,
        /// Maximum accepted UTF-16 code units.
        max: usize,
    },
    /// A substituted macro command does not parse in the live command graph.
    InvalidCommand {
        /// Function identifier.
        function: Identifier,
        /// Source line containing the macro template.
        line: usize,
        /// Substituted command text.
        command: String,
        /// Underlying command parse error.
        source: CommandParseError,
    },
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
            CommandFunctionParseErrorKind::InvalidMacro { message } => {
                write!(f, "invalid command function macro: {message}")
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
    /// The line starts with `$`, but the macro template is invalid.
    InvalidMacro {
        /// Template parse failure.
        message: String,
    },
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
    ///
    /// Vanilla builds function tags against the loaded function map and skips a
    /// tag when any required reference is missing. Steel rejects such tags at
    /// insertion so resolution cannot silently drop entries later.
    ///
    /// # Errors
    ///
    /// Returns the missing function IDs when the tag references unloaded
    /// functions.
    pub fn insert_tag(
        &mut self,
        id: Identifier,
        functions: Vec<Identifier>,
    ) -> Result<(), CommandFunctionTagError> {
        let missing_functions = functions
            .iter()
            .filter(|function| !self.functions.contains_key(*function))
            .cloned()
            .collect::<Vec<_>>();
        if !missing_functions.is_empty() {
            return Err(CommandFunctionTagError::for_missing_functions(
                id,
                missing_functions,
            ));
        }
        self.tags.insert(id, functions);
        Ok(())
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
    ///
    /// Unknown tags resolve to an empty list, matching vanilla's server
    /// function library. Unknown direct function IDs are errors.
    ///
    /// # Errors
    ///
    /// Returns an error for an unknown direct function ID.
    pub fn resolve_argument(
        &self,
        argument: &CommandFunctionArgumentValue,
    ) -> Result<Vec<CommandFunction>, CommandFunctionResolveError> {
        match argument {
            CommandFunctionArgumentValue::Function(id) => self
                .function(id)
                .map(|function| vec![function])
                .ok_or_else(|| CommandFunctionResolveError::unknown_function(id.clone())),
            CommandFunctionArgumentValue::Tag(id) => Ok(self.tag(id)),
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

/// Error returned when registering a command function tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandFunctionTagError {
    tag: Identifier,
    missing_functions: Vec<Identifier>,
}

impl CommandFunctionTagError {
    fn for_missing_functions(tag: Identifier, missing_functions: Vec<Identifier>) -> Self {
        Self {
            tag,
            missing_functions,
        }
    }

    /// Returns the tag that failed registration.
    #[must_use]
    pub const fn tag(&self) -> &Identifier {
        &self.tag
    }

    /// Returns missing required function IDs.
    #[must_use]
    pub fn missing_functions(&self) -> &[Identifier] {
        &self.missing_functions
    }
}

impl fmt::Display for CommandFunctionTagError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "command function tag {} references missing functions",
            self.tag
        )
    }
}

impl Error for CommandFunctionTagError {}

/// Error returned when resolving a parsed command function argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandFunctionResolveError {
    kind: CommandFunctionResolveErrorKind,
}

impl CommandFunctionResolveError {
    fn unknown_function(id: Identifier) -> Self {
        Self {
            kind: CommandFunctionResolveErrorKind::UnknownFunction(id),
        }
    }

    /// Returns the specific resolution failure.
    #[must_use]
    pub const fn kind(&self) -> &CommandFunctionResolveErrorKind {
        &self.kind
    }
}

impl fmt::Display for CommandFunctionResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            CommandFunctionResolveErrorKind::UnknownFunction(id) => {
                write!(f, "unknown command function {id}")
            }
        }
    }
}

impl Error for CommandFunctionResolveError {}

/// Specific command function resolution failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandFunctionResolveErrorKind {
    /// A direct function ID was not registered.
    UnknownFunction(Identifier),
}

fn sorted_ids<'a>(ids: impl Iterator<Item = &'a Identifier>) -> Vec<&'a Identifier> {
    let mut ids = ids.collect::<Vec<_>>();
    ids.sort_by_cached_key(|id| id.to_string());
    ids
}

struct ParsedFunctionSource {
    commands: Vec<String>,
    entries: Vec<CommandFunctionEntry>,
    source_lines: Vec<usize>,
    macro_parameters: Vec<String>,
}

fn parse_function_source(source: &str) -> Result<ParsedFunctionSource, CommandFunctionParseError> {
    let lines = source.lines().collect::<Vec<_>>();
    let mut commands = Vec::new();
    let mut entries = Vec::new();
    let mut source_lines = Vec::new();
    let mut macro_parameters = Vec::new();
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
        let entry =
            parse_command_function_line(&line, line_number, commands.len(), &mut macro_parameters)?;
        commands.push(line);
        entries.push(entry);
        source_lines.push(line_number);
    }

    Ok(ParsedFunctionSource {
        commands,
        entries,
        source_lines,
        macro_parameters,
    })
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

fn parse_command_function_line(
    line: &str,
    line_number: usize,
    command_index: usize,
    macro_parameters: &mut Vec<String>,
) -> Result<CommandFunctionEntry, CommandFunctionParseError> {
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
    let Some(template_source) = line.strip_prefix('$') else {
        return Ok(CommandFunctionEntry::Plain { command_index });
    };

    let template = MacroTemplate::parse(template_source).map_err(|message| {
        CommandFunctionParseError::new(
            line_number,
            CommandFunctionParseErrorKind::InvalidMacro { message },
        )
    })?;
    let parameter_indices = template
        .variables
        .iter()
        .map(|variable| macro_parameter_index(macro_parameters, variable))
        .collect::<Vec<_>>()
        .into();

    Ok(CommandFunctionEntry::Macro {
        template,
        parameter_indices,
    })
}

fn macro_parameter_index(parameters: &mut Vec<String>, variable: &str) -> usize {
    if let Some(index) = parameters
        .iter()
        .position(|parameter| parameter == variable)
    {
        return index;
    }
    let index = parameters.len();
    parameters.push(variable.to_owned());
    index
}

impl MacroTemplate {
    fn parse(input: &str) -> Result<Self, String> {
        let mut segments = Vec::new();
        let mut variables = Vec::new();
        let mut start = 0;
        let mut search_start = 0;
        while let Some(relative_index) = input[search_start..].find('$') {
            let index = search_start + relative_index;
            if input[index + 1..].starts_with('(') {
                segments.push(input[start..index].to_owned());
                let variable_start = index + 2;
                let Some(relative_end) = input[variable_start..].find(')') else {
                    return Err("unterminated macro variable".to_owned());
                };
                let variable_end = variable_start + relative_end;
                let variable = &input[variable_start..variable_end];
                if !is_valid_macro_variable(variable) {
                    return Err(format!("invalid macro variable name '{variable}'"));
                }
                variables.push(variable.to_owned());
                start = variable_end + 1;
                search_start = start;
            } else {
                search_start = index + 1;
            }
        }

        if start == 0 {
            return Err("no variables in macro".to_owned());
        }
        if start != input.len() {
            segments.push(input[start..].to_owned());
        }

        Ok(Self {
            segments: segments.into(),
            variables: variables.into(),
        })
    }

    fn substitute<'a>(
        &self,
        substitutions: impl IntoIterator<Item = &'a str>,
    ) -> Result<String, CommandFunctionInstantiationError> {
        let mut output = String::new();
        for (segment, substitution) in self.segments.iter().zip(substitutions) {
            output.push_str(segment);
            output.push_str(substitution);
            check_instantiated_command_line_length(&output)?;
        }
        if self.segments.len() > self.variables.len() {
            if let Some(segment) = self.segments.last() {
                output.push_str(segment);
            }
        }
        check_instantiated_command_line_length(&output)?;
        Ok(output)
    }
}

fn is_valid_macro_variable(variable: &str) -> bool {
    variable.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
}

fn check_instantiated_command_line_length(
    line: &str,
) -> Result<(), CommandFunctionInstantiationError> {
    let length = line.encode_utf16().count();
    if length > MAX_COMMAND_FUNCTION_LINE_LENGTH {
        return Err(CommandFunctionInstantiationError::command_too_long(
            length,
            MAX_COMMAND_FUNCTION_LINE_LENGTH,
        ));
    }
    Ok(())
}

fn stringify_macro_argument(tag: &NbtTag) -> String {
    match tag {
        NbtTag::Byte(value) => value.to_string(),
        NbtTag::Short(value) => value.to_string(),
        NbtTag::Int(value) => value.to_string(),
        NbtTag::Long(value) => value.to_string(),
        NbtTag::Float(value) => format_decimal_f32(*value),
        NbtTag::Double(value) => format_decimal_f64(*value),
        NbtTag::String(value) => value.to_str().into_owned(),
        NbtTag::ByteArray(_)
        | NbtTag::List(_)
        | NbtTag::Compound(_)
        | NbtTag::IntArray(_)
        | NbtTag::LongArray(_) => snbt_tag(tag),
    }
}

fn snbt_tag(tag: &NbtTag) -> String {
    match tag {
        NbtTag::Byte(value) => format!("{value}b"),
        NbtTag::Short(value) => format!("{value}s"),
        NbtTag::Int(value) => value.to_string(),
        NbtTag::Long(value) => format!("{value}l"),
        NbtTag::Float(value) => format!("{}f", format_decimal_f32(*value)),
        NbtTag::Double(value) => format!("{}d", format_decimal_f64(*value)),
        NbtTag::ByteArray(values) => format!(
            "[B;{}]",
            values
                .iter()
                .map(|value| format!("{}b", *value as i8))
                .collect::<Vec<_>>()
                .join(",")
        ),
        NbtTag::String(value) => quote_snbt_string(&value.to_str()),
        NbtTag::List(list) => snbt_list(list),
        NbtTag::Compound(compound) => snbt_compound(compound),
        NbtTag::IntArray(values) => {
            format!(
                "[I;{}]",
                values
                    .iter()
                    .map(i32::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        NbtTag::LongArray(values) => format!(
            "[L;{}]",
            values
                .iter()
                .map(|value| format!("{value}l"))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

fn snbt_list(list: &NbtList) -> String {
    format!(
        "[{}]",
        list.as_nbt_tags()
            .iter()
            .map(snbt_tag)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn snbt_compound(compound: &NbtCompound) -> String {
    format!(
        "{{{}}}",
        compound
            .iter()
            .map(|(key, value)| format!("{}:{}", snbt_key(&key.to_str()), snbt_tag(value)))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn snbt_key(key: &str) -> String {
    if key
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '+'))
    {
        return key.to_owned();
    }
    quote_snbt_string(key)
}

fn quote_snbt_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            _ => output.push(ch),
        }
    }
    output.push('"');
    output
}

fn format_decimal_f32(value: f32) -> String {
    format_decimal_f64(f64::from(value))
}

fn format_decimal_f64(value: f64) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    let mut formatted = format!("{value:.15}");
    if let Some(decimal) = formatted.find('.') {
        let trim_start = decimal + 1;
        while formatted.len() > trim_start && formatted.ends_with('0') {
            formatted.pop();
        }
        if formatted.ends_with('.') {
            formatted.pop();
        }
    }
    if formatted == "-0" {
        return "0".to_owned();
    }
    formatted
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
    use simdnbt::owned::{NbtCompound, NbtTag};
    use steel_utils::Identifier;

    use crate::{
        command::{
            CommandDispatcher, CommandRegistration,
            graph::{
                CommandFunctionArgumentValue, CommandParseErrorKind, CommandResult, StringParser,
                argument, literal,
            },
            parsers::{EntityParser, ScoreHolderParser},
            reader::StringMode,
            requirement::{
                CommandInputContext, CommandSourceKind, PermissionExpr, RequirementContext,
            },
        },
        permission::PermissionSegment,
    };

    use super::{
        CommandFunction, CommandFunctionInstantiationErrorKind, CommandFunctionParseErrorKind,
        CommandFunctionRegistry, CommandFunctionResolveErrorKind, MAX_COMMAND_FUNCTION_LINE_LENGTH,
    };

    struct TestContext;

    impl RequirementContext for TestContext {
        fn source_kind(&self) -> CommandSourceKind {
            CommandSourceKind::Console
        }

        fn has_permission(&self, _permission: &PermissionExpr) -> bool {
            true
        }
    }

    impl CommandInputContext for TestContext {}

    fn validation_dispatcher() -> CommandDispatcher {
        let mut dispatcher = CommandDispatcher::new_empty();
        let namespace = PermissionSegment::parse("test").expect("namespace parses");
        let registration = CommandRegistration::new(
            literal("known").executes(|_, _| Ok(CommandResult::success())),
            namespace,
        )
        .public();
        let targeted_registration = CommandRegistration::new(
            literal("targeted").then(
                argument("targets", EntityParser::multiple())
                    .executes(|_, _| Ok(CommandResult::success())),
            ),
            PermissionSegment::parse("test").expect("namespace parses"),
        )
        .public();
        let score_targeted_registration = CommandRegistration::new(
            literal("scoretarget").then(
                argument("holders", ScoreHolderParser::multiple())
                    .executes(|_, _| Ok(CommandResult::success())),
            ),
            PermissionSegment::parse("test").expect("namespace parses"),
        )
        .public();
        let echo_registration = CommandRegistration::new(
            literal("echo").then(
                argument("message", StringParser::new(StringMode::GreedyPhrase))
                    .executes(|_, _| Ok(CommandResult::success())),
            ),
            PermissionSegment::parse("test").expect("namespace parses"),
        )
        .public();

        dispatcher
            .register_command(registration)
            .expect("command registers");
        dispatcher
            .register_command(targeted_registration)
            .expect("targeted command registers");
        dispatcher
            .register_command(score_targeted_registration)
            .expect("score-targeted command registers");
        dispatcher
            .register_command(echo_registration)
            .expect("echo command registers");
        dispatcher
    }

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
        registry
            .insert_tag(tag.clone(), vec![second.clone(), first.clone()])
            .expect("tag references loaded functions");

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
                .expect("function resolves")
                .into_iter()
                .map(|function| function.id)
                .collect::<Vec<_>>(),
            vec![first]
        );
        assert_eq!(
            registry
                .resolve_argument(&CommandFunctionArgumentValue::Tag(tag.clone()))
                .expect("tag resolves")
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
        assert_eq!(
            registry
                .resolve_argument(&CommandFunctionArgumentValue::Tag(Identifier::new_static(
                    "test", "missing"
                )))
                .expect("unknown tag resolves to an empty collection"),
            Vec::new()
        );
    }

    #[test]
    fn registry_rejects_tags_with_missing_required_functions() {
        let loaded = Identifier::new_static("test", "loaded");
        let missing = Identifier::new_static("test", "missing");
        let tag = Identifier::new_static("test", "load");
        let mut registry = CommandFunctionRegistry::default();
        registry.insert_function(CommandFunction::new(
            loaded.clone(),
            vec!["say loaded".to_owned()],
        ));

        let error = registry
            .insert_tag(tag.clone(), vec![loaded, missing.clone()])
            .expect_err("tag with a missing function rejects");

        assert_eq!(error.tag(), &tag);
        assert_eq!(error.missing_functions(), &[missing]);
        assert!(
            registry
                .tag(&Identifier::new_static("test", "load"))
                .is_empty()
        );
    }

    #[test]
    fn registry_errors_for_unknown_direct_function_arguments() {
        let registry = CommandFunctionRegistry::default();
        let missing = Identifier::new_static("test", "missing");

        let error = registry
            .resolve_argument(&CommandFunctionArgumentValue::Function(missing.clone()))
            .expect_err("unknown direct function rejects");

        assert!(matches!(
            error.kind(),
            CommandFunctionResolveErrorKind::UnknownFunction(id) if id == &missing
        ));
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
    fn function_source_parser_tracks_physical_command_lines() {
        let function = CommandFunction::from_source(
            Identifier::new_static("test", "load"),
            "# comment\nknown\n\nknown \\\n tail\n",
        )
        .expect("function source parses");

        assert_eq!(function.commands(), ["known", "known tail"]);
        assert_eq!(function.command_source_line(0), Some(2));
        assert_eq!(function.command_source_line(1), Some(4));
    }

    #[test]
    fn function_validation_reports_source_line() {
        let dispatcher = validation_dispatcher();
        let function = CommandFunction::from_source(
            Identifier::new_static("test", "load"),
            "# comment\nknown\n\nmissing value\n",
        )
        .expect("function source parses");

        let error = function
            .validate_commands(&dispatcher, &TestContext)
            .expect_err("unknown command should fail validation");

        assert_eq!(error.line(), 4);
        assert_eq!(error.command(), "missing value");
        assert!(matches!(
            error.source().kind(),
            CommandParseErrorKind::UnknownCommand
        ));
    }

    #[test]
    fn function_validation_keeps_entity_targets_deferred() {
        let dispatcher = validation_dispatcher();
        let function =
            CommandFunction::from_source(Identifier::new_static("test", "load"), "targeted @a\n")
                .expect("function source parses");

        function
            .validate_commands(&dispatcher, &TestContext)
            .expect("selector target validation should not require live server resolution");
    }

    #[test]
    fn function_validation_keeps_score_holder_selectors_deferred() {
        let dispatcher = validation_dispatcher();
        let function = CommandFunction::from_source(
            Identifier::new_static("test", "load"),
            "scoretarget @a\n",
        )
        .expect("function source parses");

        function
            .validate_commands(&dispatcher, &TestContext)
            .expect("score-holder selector validation should not require live server resolution");
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
    fn function_source_parser_accepts_macro_templates() {
        let function = CommandFunction::from_source(
            Identifier::new_static("test", "macro"),
            "known\n$echo $(value)\n",
        )
        .expect("macro function source parses");

        assert!(function.is_macro());
        assert_eq!(function.commands(), ["known", "$echo $(value)"]);
        assert_eq!(function.command_source_line(1), Some(2));
    }

    #[test]
    fn function_source_parser_rejects_macro_without_variables() {
        let error =
            CommandFunction::from_source(Identifier::new_static("test", "bad"), "$echo value")
                .expect_err("macro without variables rejects");

        assert!(matches!(
            error.kind(),
            CommandFunctionParseErrorKind::InvalidMacro { message }
                if message == "no variables in macro"
        ));
    }

    #[test]
    fn function_macro_instantiates_with_compound_arguments() {
        let dispatcher = validation_dispatcher();
        let function = CommandFunction::from_source(
            Identifier::new_static("test", "macro"),
            "known\n$echo $(value)\n",
        )
        .expect("macro function source parses");
        function
            .validate_commands(&dispatcher, &TestContext)
            .expect("plain commands validate while macros stay deferred");

        let mut arguments = NbtCompound::new();
        arguments.insert("value", NbtTag::String("hello world".into()));
        let instantiated = function
            .instantiate(Some(&arguments), &dispatcher, &TestContext)
            .expect("macro instantiates");

        assert_eq!(instantiated.commands(), ["known", "echo hello world"]);
    }

    #[test]
    fn function_macro_requires_arguments() {
        let dispatcher = validation_dispatcher();
        let function = CommandFunction::from_source(
            Identifier::new_static("test", "macro"),
            "$echo $(value)\n",
        )
        .expect("macro function source parses");

        let error = function
            .instantiate(None, &dispatcher, &TestContext)
            .expect_err("macro requires arguments");

        assert!(matches!(
            error.kind(),
            CommandFunctionInstantiationErrorKind::MissingArguments { function }
                if function == &Identifier::new_static("test", "macro")
        ));
    }

    #[test]
    fn function_macro_requires_each_referenced_argument() {
        let dispatcher = validation_dispatcher();
        let function = CommandFunction::from_source(
            Identifier::new_static("test", "macro"),
            "$echo $(value)\n",
        )
        .expect("macro function source parses");
        let arguments = NbtCompound::new();

        let error = function
            .instantiate(Some(&arguments), &dispatcher, &TestContext)
            .expect_err("macro requires referenced argument");

        assert!(matches!(
            error.kind(),
            CommandFunctionInstantiationErrorKind::MissingArgument { function, argument }
                if function == &Identifier::new_static("test", "macro") && argument == "value"
        ));
    }

    #[test]
    fn function_macro_validates_substituted_commands() {
        let dispatcher = validation_dispatcher();
        let function =
            CommandFunction::from_source(Identifier::new_static("test", "macro"), "$$(command)\n")
                .expect("macro function source parses");
        let mut arguments = NbtCompound::new();
        arguments.insert("command", NbtTag::String("missing value".into()));

        let error = function
            .instantiate(Some(&arguments), &dispatcher, &TestContext)
            .expect_err("substituted command must parse");

        assert!(matches!(
            error.kind(),
            CommandFunctionInstantiationErrorKind::InvalidCommand {
                function,
                line: 1,
                command,
                source,
            } if function == &Identifier::new_static("test", "macro")
                && command == "missing value"
                && matches!(source.kind(), CommandParseErrorKind::UnknownCommand)
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
