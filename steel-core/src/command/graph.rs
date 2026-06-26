//! Dynamic command graph, parsers, and structured parse results.

use std::{error::Error, fmt, sync::Arc};

use steel_protocol::packets::game::{CommandNode as ProtocolCommandNode, SuggestionEntry};

use crate::command::{
    CommandDispatcher,
    context::CommandContext,
    error::CommandError,
    reader::CommandReader,
    requirement::{CommandInputContext, RequirementContext},
};
use crate::permission::{PermissionExpr, PermissionKey, PermissionKeyError};

mod arguments;
mod builder;
mod node;
mod primitive_parsers;
mod traversal;

pub use arguments::{
    CommandPermissionArgument, FromParsedArgument, ParsedArgument, ParsedArgumentError,
    ParsedArguments, PermissionTarget, StructureArgumentValue,
};
pub use builder::{CommandNodeBuilder, argument, literal};
use node::{CommandNode, CommandNodeKind, merge_or_push_node};
pub use primitive_parsers::{
    AnchorParser, BoolParser, CommandArgumentParser, FloatParser, IntegerParser, StringParser,
};
use traversal::{parse_children, suggest_children};

/// Structured command parse error.
#[derive(Clone, Debug, PartialEq)]
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

    const fn is_better_than(&self, other: &Self) -> bool {
        self.cursor > other.cursor
            || self.cursor == other.cursor && self.kind.precedence() > other.kind.precedence()
    }
}

/// Specific command parse error kind.
#[derive(Clone, Debug, PartialEq)]
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
    /// An entity anchor argument was invalid.
    InvalidAnchor(String),
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
    /// A float argument was invalid.
    InvalidFloat(String),
    /// A float argument was below its minimum.
    FloatTooLow {
        /// Parsed value.
        value: f32,
        /// Minimum accepted value.
        min: f32,
    },
    /// A float argument was above its maximum.
    FloatTooHigh {
        /// Parsed value.
        value: f32,
        /// Maximum accepted value.
        max: f32,
    },
    /// A game mode argument was invalid.
    InvalidGameMode(String),
    /// A player argument was invalid.
    InvalidPlayer(String),
    /// An entity argument was invalid.
    InvalidEntity(String),
    /// An entity type argument was invalid.
    InvalidEntityType(String),
    /// An item argument was invalid.
    InvalidItem(String),
    /// An enchantment argument was invalid.
    InvalidEnchantment(String),
    /// A structure argument was invalid.
    InvalidStructure(String),
    /// A domain argument was invalid.
    InvalidDomain(String),
    /// A world argument was invalid.
    InvalidWorld(String),
    /// A 3D vector argument was invalid.
    InvalidVec3(String),
    /// A block position argument was invalid.
    InvalidBlockPos(String),
    /// A rotation argument was invalid.
    InvalidRotation(String),
    /// A text component argument was invalid.
    InvalidComponent(String),
    /// A time argument was invalid.
    InvalidTime(String),
    /// A permission key argument was invalid.
    InvalidPermissionKey(String),
    /// A permission group argument was invalid.
    InvalidPermissionGroup(String),
    /// A parser required live command context that was not available.
    MissingCommandContext(&'static str),
}

impl CommandParseErrorKind {
    const fn precedence(&self) -> u8 {
        match self {
            Self::TrailingData => 7,
            Self::InvalidBool(_)
            | Self::InvalidAnchor(_)
            | Self::InvalidInteger(_)
            | Self::IntegerTooLow { .. }
            | Self::IntegerTooHigh { .. }
            | Self::InvalidFloat(_)
            | Self::FloatTooLow { .. }
            | Self::FloatTooHigh { .. }
            | Self::InvalidGameMode(_)
            | Self::InvalidPlayer(_)
            | Self::InvalidEntity(_)
            | Self::InvalidEntityType(_)
            | Self::InvalidItem(_)
            | Self::InvalidEnchantment(_)
            | Self::InvalidStructure(_)
            | Self::InvalidDomain(_)
            | Self::InvalidWorld(_)
            | Self::InvalidVec3(_)
            | Self::InvalidBlockPos(_)
            | Self::InvalidRotation(_)
            | Self::InvalidComponent(_)
            | Self::InvalidTime(_)
            | Self::InvalidPermissionKey(_)
            | Self::InvalidPermissionGroup(_)
            | Self::MissingCommandContext(_)
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

/// Invalid command graph registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandGraphError {
    /// A literal node name is not parseable as one command token.
    InvalidLiteralName {
        /// Invalid literal name.
        name: String,
        /// Validation failure.
        source: CommandNodeNameError,
    },
    /// An argument node name is invalid for protocol exposure and parsed argument lookup.
    InvalidArgumentName {
        /// Invalid argument name.
        name: String,
        /// Validation failure.
        source: CommandNodeNameError,
    },
    /// A literal node collided with an existing literal that cannot be merged.
    LiteralCollision {
        /// Colliding literal name.
        name: String,
    },
    /// A derived subcommand permission was requested on a non-literal node.
    DerivedPermissionRequiresLiteral {
        /// Node name.
        name: String,
    },
    /// A derived subcommand permission produced an invalid permission key.
    InvalidDerivedPermissionKey(PermissionKeyError),
    /// Dynamic argument permissions require command registration metadata.
    UnresolvedDynamicPermission {
        /// Node name.
        name: String,
    },
    /// Derived subcommand permissions require command registration metadata.
    UnresolvedDerivedPermission {
        /// Node name.
        name: String,
    },
    /// A dynamic argument permission referenced an argument that is not available.
    MissingDynamicPermissionArgument {
        /// Node name.
        node: String,
        /// Missing argument name.
        argument: String,
    },
    /// A dynamic argument permission referenced an argument with the wrong parsed type.
    WrongDynamicPermissionArgumentType {
        /// Node name.
        node: String,
        /// Argument name.
        argument: String,
        /// Type expected by the permission marker.
        expected: &'static str,
        /// Type produced by the parser.
        actual: &'static str,
    },
}

impl fmt::Display for CommandGraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLiteralName { name, source } => {
                write!(f, "invalid command literal '{name}': {source}")
            }
            Self::InvalidArgumentName { name, source } => {
                write!(f, "invalid command argument '{name}': {source}")
            }
            Self::LiteralCollision { name } => {
                write!(
                    f,
                    "command literal '{name}' is already registered differently"
                )
            }
            Self::DerivedPermissionRequiresLiteral { name } => {
                write!(
                    f,
                    "derived subcommand permission requires a literal node, got '{name}'"
                )
            }
            Self::InvalidDerivedPermissionKey(error) => write!(f, "{error}"),
            Self::UnresolvedDynamicPermission { name } => {
                write!(
                    f,
                    "dynamic permission on command node '{name}' was not resolved during registration"
                )
            }
            Self::UnresolvedDerivedPermission { name } => {
                write!(
                    f,
                    "derived permission on command node '{name}' was not resolved during registration"
                )
            }
            Self::MissingDynamicPermissionArgument { node, argument } => {
                write!(
                    f,
                    "dynamic permission on command node '{node}' references unavailable argument '{argument}'"
                )
            }
            Self::WrongDynamicPermissionArgumentType {
                node,
                argument,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "dynamic permission on command node '{node}' references argument '{argument}' as {expected}, but parser stores {actual}"
                )
            }
        }
    }
}

impl From<PermissionKeyError> for CommandGraphError {
    fn from(value: PermissionKeyError) -> Self {
        Self::InvalidDerivedPermissionKey(value)
    }
}

impl Error for CommandGraphError {}

/// Invalid command node name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandNodeNameError {
    /// The name is empty.
    Empty,
    /// The name contains whitespace, which cannot be matched as one command token.
    ContainsWhitespace,
}

impl fmt::Display for CommandNodeNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "name is empty"),
            Self::ContainsWhitespace => write!(f, "name contains whitespace"),
        }
    }
}

impl Error for CommandNodeNameError {}

pub(crate) fn validate_command_node_name(name: &str) -> Result<(), CommandNodeNameError> {
    if name.is_empty() {
        return Err(CommandNodeNameError::Empty);
    }
    if name.chars().any(char::is_whitespace) {
        return Err(CommandNodeNameError::ContainsWhitespace);
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DynamicPermissionError {
    ParsedArgument(ParsedArgumentError),
    PermissionKey(PermissionKeyError),
}

impl fmt::Display for DynamicPermissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParsedArgument(error) => write!(f, "{error:?}"),
            Self::PermissionKey(error) => write!(f, "{error}"),
        }
    }
}

impl From<ParsedArgumentError> for DynamicPermissionError {
    fn from(value: ParsedArgumentError) -> Self {
        Self::ParsedArgument(value)
    }
}

impl From<PermissionKeyError> for DynamicPermissionError {
    fn from(value: PermissionKeyError) -> Self {
        Self::PermissionKey(value)
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
type UnresolvedDynamicPermissionResolver = Arc<
    dyn Fn(&PermissionKey, &ParsedArguments) -> Result<PermissionExpr, DynamicPermissionError>
        + Send
        + Sync,
>;
type DynamicPermissionResolver =
    Arc<dyn Fn(&ParsedArguments) -> Result<PermissionExpr, DynamicPermissionError> + Send + Sync>;

#[derive(Clone)]
struct UnresolvedDynamicPermission {
    argument_name: String,
    expected_type: &'static str,
    catalog_segments: &'static [&'static str],
    resolver: UnresolvedDynamicPermissionResolver,
}

impl UnresolvedDynamicPermission {
    fn argument<T>(name: String) -> Self
    where
        T: CommandPermissionArgument + 'static,
    {
        let argument_name = name;
        Self {
            argument_name: argument_name.clone(),
            expected_type: T::TYPE_NAME,
            catalog_segments: T::catalog_permission_segments(),
            resolver: Arc::new(move |base_permission, arguments| {
                let value = arguments.get::<T>(&argument_name)?;
                let segment = value.permission_segment()?;
                Ok(PermissionExpr::key(base_permission.child(&segment)?))
            }),
        }
    }

    fn resolve(
        self,
        root_permission: &PermissionKey,
        base_permission: &PermissionKey,
    ) -> DynamicPermission {
        let root_permission = root_permission.to_owned();
        let base_permission = base_permission.to_owned();
        let resolver = self.resolver;
        DynamicPermission {
            resolver: Arc::new(move |arguments| {
                Ok(PermissionExpr::key(root_permission.clone())
                    | resolver(&base_permission, arguments)?)
            }),
        }
    }
}

#[derive(Clone)]
struct DynamicPermission {
    resolver: DynamicPermissionResolver,
}

impl DynamicPermission {
    fn expression(
        &self,
        arguments: &ParsedArguments,
    ) -> Result<PermissionExpr, DynamicPermissionError> {
        (self.resolver)(arguments)
    }
}

fn dynamic_permissions_allow(
    permissions: &[DynamicPermission],
    arguments: &ParsedArguments,
    context: &dyn RequirementContext,
) -> Result<bool, DynamicPermissionError> {
    for permission in permissions {
        let expression = permission.expression(arguments)?;
        if !context.has_permission(&expression) {
            return Ok(false);
        }
    }

    Ok(true)
}

/// Target for redirecting graph execution after a node action runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandRedirectTarget {
    /// Redirects back to the current root command.
    Current,
    /// Redirects to the command dispatcher root.
    All,
}

#[derive(Clone)]
enum ParsedCommandAction {
    Execute(CommandExecutor),
    Redirect(ParsedRedirect),
}

#[derive(Clone)]
struct ParsedRedirect {
    target: CommandRedirectTarget,
    current_root: String,
    command: String,
    executor: CommandExecutor,
}

/// A successfully parsed command.
#[derive(Clone)]
pub struct ParseResults {
    input: String,
    arguments: ParsedArguments,
    path: Vec<String>,
    dynamic_permissions: Vec<DynamicPermission>,
    action: ParsedCommandAction,
}

impl fmt::Debug for ParseResults {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
        self.check_dynamic_permissions(context)?;
        match &self.action {
            ParsedCommandAction::Execute(executor) => executor(context, &self.arguments),
            ParsedCommandAction::Redirect(redirect) => {
                (redirect.executor)(context, &self.arguments)?;
                let command = match redirect.target {
                    CommandRedirectTarget::Current => {
                        format!("{} {}", redirect.current_root, redirect.command)
                    }
                    CommandRedirectTarget::All => redirect.command.clone(),
                };
                Err(CommandError::failure(format!(
                    "Command redirect target '{command}' is unavailable"
                )))
            }
        }
    }

    /// Executes this parsed command, using `dispatcher` for redirected command tails.
    ///
    /// # Errors
    ///
    /// Returns an error from either this command's executor or the redirected command.
    pub(crate) fn execute_with_dispatcher(
        &self,
        context: &mut CommandContext,
        dispatcher: &CommandDispatcher,
    ) -> Result<CommandResult, CommandError> {
        self.check_dynamic_permissions(context)?;
        match &self.action {
            ParsedCommandAction::Execute(executor) => executor(context, &self.arguments),
            ParsedCommandAction::Redirect(redirect) => {
                (redirect.executor)(context, &self.arguments)?;
                match redirect.target {
                    CommandRedirectTarget::Current => {
                        let command = format!("{} {}", redirect.current_root, redirect.command);
                        dispatcher.dispatch_with_context(command, context)
                    }
                    CommandRedirectTarget::All => {
                        dispatcher.dispatch_with_context(redirect.command.clone(), context)
                    }
                }
            }
        }
    }

    fn check_dynamic_permissions(
        &self,
        context: &dyn RequirementContext,
    ) -> Result<(), CommandError> {
        if dynamic_permissions_allow(&self.dynamic_permissions, &self.arguments, context).map_err(
            |error| CommandError::InvalidConsumption(Some(format!("dynamic permission: {error}"))),
        )? {
            Ok(())
        } else {
            Err(CommandError::PermissionDenied)
        }
    }
}

/// Dynamic command graph.
#[derive(Clone, Default)]
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
    ///
    /// # Errors
    ///
    /// Returns an error when the node tree contains invalid names or collides
    /// with an existing literal that cannot be merged.
    pub fn with_root(mut self, root: CommandNodeBuilder) -> Result<Self, CommandGraphError> {
        self.register_root(root)?;
        Ok(self)
    }

    /// Adds a root command node.
    ///
    /// # Errors
    ///
    /// Returns an error when the node tree contains invalid names or collides
    /// with an existing literal that cannot be merged.
    pub fn register_root(&mut self, root: CommandNodeBuilder) -> Result<(), CommandGraphError> {
        merge_or_push_node(&mut self.roots, root.build()?)
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
            root.usage(buffer, root_children, context, None);
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
        let reader = CommandReader::with_offset(input_without_slash, cursor_offset);

        suggest_children(
            &reader,
            &self.roots,
            ParsedArguments::default(),
            Vec::new(),
            context,
            &self.roots,
            None,
        )
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
            Vec::new(),
            context,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use crate::command::commands;
    use crate::command::parsers::GameModeParser;
    use crate::command::{
        graph::{
            AnchorParser, BoolParser, CommandGraph, CommandGraphError, CommandNodeBuilder,
            CommandNodeNameError, CommandParseErrorKind, CommandRedirectTarget, CommandResult,
            FloatParser, IntegerParser, ParsedCommandAction, StringParser, SuggestionResult,
            argument, literal,
        },
        reader::StringMode,
        requirement::{
            CommandInputContext, CommandSourceKind, PermissionExpr, PermissionKey, Requirement,
            RequirementContext,
        },
    };
    use crate::permission::{
        PermissionCatalog, PermissionCatalogSource, PermissionEntry, PermissionSet,
    };
    use steel_protocol::packets::game::CommandNode as ProtocolCommandNode;
    use steel_utils::{serial::WriteTo, types::GameType};

    struct TestContext {
        source_kind: CommandSourceKind,
        permissions: PermissionSet,
    }

    impl RequirementContext for TestContext {
        fn source_kind(&self) -> CommandSourceKind {
            self.source_kind
        }

        fn has_permission(&self, permission: &PermissionExpr) -> bool {
            self.permissions.allows(permission)
        }
    }

    impl CommandInputContext for TestContext {}

    fn player_context() -> TestContext {
        TestContext {
            source_kind: CommandSourceKind::Player,
            permissions: PermissionSet::default(),
        }
    }

    fn player_context_with(permission: PermissionKey) -> TestContext {
        player_context_with_all([permission])
    }

    fn player_context_with_all<const N: usize>(permissions: [PermissionKey; N]) -> TestContext {
        TestContext {
            source_kind: CommandSourceKind::Player,
            permissions: PermissionSet::from_entries(permissions.map(PermissionEntry::allow)),
        }
    }

    fn suggestion_texts(result: &SuggestionResult) -> Vec<String> {
        result
            .suggestions
            .iter()
            .map(|suggestion| suggestion.text.clone())
            .collect()
    }

    fn graph_with_root(root: CommandNodeBuilder) -> CommandGraph {
        CommandGraph::new()
            .with_root(root)
            .expect("test command root registers")
    }

    fn graph_registration_error(root: CommandNodeBuilder) -> CommandGraphError {
        match CommandGraph::new().with_root(root) {
            Ok(_) => panic!("test command root should be rejected"),
            Err(error) => error,
        }
    }

    #[test]
    fn registration_rejects_invalid_node_names() {
        let error = graph_registration_error(literal("bad name"));
        assert_eq!(
            error,
            CommandGraphError::InvalidLiteralName {
                name: "bad name".to_owned(),
                source: CommandNodeNameError::ContainsWhitespace
            }
        );

        let error = graph_registration_error(literal("root").then(argument("", BoolParser)));
        assert_eq!(
            error,
            CommandGraphError::InvalidArgumentName {
                name: String::new(),
                source: CommandNodeNameError::Empty
            }
        );
    }

    #[test]
    fn registration_rejects_unmergeable_literal_collisions() {
        let graph = graph_with_root(literal("root").executes(|_, _| Ok(CommandResult::success())));
        let Err(error) =
            graph.with_root(literal("root").executes(|_, _| Ok(CommandResult::success())))
        else {
            panic!("duplicate executable root should be rejected");
        };

        assert_eq!(
            error,
            CommandGraphError::LiteralCollision {
                name: "root".to_owned()
            }
        );
    }

    #[test]
    fn derived_subcommand_permission_requires_literal_node() {
        let root_permission =
            PermissionKey::parse("minecraft.command.root").expect("permission key parses");
        let root =
            literal("root").then(argument("target", BoolParser).requires_subcommand_permission());
        let mut catalog = PermissionCatalog::new();
        let Err(error) = root.resolve_subcommand_permissions(&root_permission, &mut catalog) else {
            panic!("argument node permission marker should be rejected");
        };

        assert_eq!(
            error,
            CommandGraphError::DerivedPermissionRequiresLiteral {
                name: "target".to_owned()
            }
        );
    }

    #[test]
    fn dynamic_argument_permission_requires_registration_resolution() {
        let error = graph_registration_error(
            literal("root").then(
                argument("gamemode", GameModeParser)
                    .requires_argument_permission::<GameType>("gamemode"),
            ),
        );

        assert_eq!(
            error,
            CommandGraphError::UnresolvedDynamicPermission {
                name: "gamemode".to_owned()
            }
        );
    }

    #[test]
    fn derived_subcommand_permission_requires_registration_resolution() {
        let error = graph_registration_error(
            literal("root").then(literal("child").requires_subcommand_permission()),
        );

        assert_eq!(
            error,
            CommandGraphError::UnresolvedDerivedPermission {
                name: "child".to_owned()
            }
        );
    }

    #[test]
    fn dynamic_argument_permission_requires_available_argument() {
        let root_permission =
            PermissionKey::parse("minecraft.command.root").expect("permission key parses");
        let mut catalog = PermissionCatalog::new();
        let Err(error) = literal("root")
            .then(
                argument("gamemode", GameModeParser)
                    .requires_argument_permission::<GameType>("mode"),
            )
            .resolve_subcommand_permissions(&root_permission, &mut catalog)
        else {
            panic!("missing dynamic permission argument should reject registration");
        };

        assert_eq!(
            error,
            CommandGraphError::MissingDynamicPermissionArgument {
                node: "gamemode".to_owned(),
                argument: "mode".to_owned()
            }
        );
    }

    #[test]
    fn dynamic_argument_permission_validates_argument_type() {
        let root_permission =
            PermissionKey::parse("minecraft.command.root").expect("permission key parses");
        let mut catalog = PermissionCatalog::new();
        let Err(error) = literal("root")
            .then(
                argument("enabled", BoolParser).requires_argument_permission::<GameType>("enabled"),
            )
            .resolve_subcommand_permissions(&root_permission, &mut catalog)
        else {
            panic!("wrong dynamic permission argument type should reject registration");
        };

        assert_eq!(
            error,
            CommandGraphError::WrongDynamicPermissionArgumentType {
                node: "enabled".to_owned(),
                argument: "enabled".to_owned(),
                expected: "gamemode",
                actual: "bool"
            }
        );
    }

    #[test]
    fn permission_resolution_registers_catalog_entries() {
        let root_permission =
            PermissionKey::parse("minecraft.command.gamemode").expect("permission key parses");
        let mut catalog = PermissionCatalog::new();
        commands::gamemode::command()
            .resolve_subcommand_permissions(&root_permission, &mut catalog)
            .expect("gamemode permissions resolve");

        let suggestions = catalog.suggestions("minecraft.command.gamemode");
        assert_eq!(
            suggestions,
            vec![
                "minecraft.command.gamemode.adventure".to_owned(),
                "minecraft.command.gamemode.creative".to_owned(),
                "minecraft.command.gamemode.spectator".to_owned(),
                "minecraft.command.gamemode.survival".to_owned(),
            ]
        );
        assert!(
            catalog
                .entries()
                .all(|entry| entry.sources().contains(&PermissionCatalogSource::Command))
        );
    }

    #[test]
    fn explicit_permission_requirements_register_catalog_entries() {
        let root_permission =
            PermissionKey::parse("minecraft.command.root").expect("permission key parses");
        let admin = PermissionKey::parse("steel.admin").expect("permission key parses");
        let audit = PermissionKey::parse("steel.audit").expect("permission key parses");
        let mut catalog = PermissionCatalog::new();

        literal("root")
            .then(
                literal("admin").requires_permission_expr(
                    PermissionExpr::key(admin) | PermissionExpr::key(audit),
                ),
            )
            .resolve_subcommand_permissions(&root_permission, &mut catalog)
            .expect("permissions resolve");

        assert_eq!(
            catalog.suggestions("steel."),
            vec!["steel.admin".to_owned(), "steel.audit".to_owned()]
        );
    }

    #[test]
    fn gamemode_allows_root_or_dynamic_value_permission() {
        let root_permission =
            PermissionKey::parse("minecraft.command.gamemode").expect("permission key parses");
        let mut catalog = PermissionCatalog::new();
        let root = commands::gamemode::command()
            .resolve_subcommand_permissions(&root_permission, &mut catalog)
            .expect("gamemode permissions resolve");
        let graph = graph_with_root(root);

        let root_only = player_context_with(root_permission.clone());
        assert!(graph.parse("gamemode creative", &root_only).is_ok());

        let creative = player_context_with(
            PermissionKey::parse("minecraft.command.gamemode.creative")
                .expect("permission key parses"),
        );
        assert!(graph.parse("gamemode creative", &creative).is_ok());
        assert!(graph.parse("gamemode survival", &creative).is_err());
    }

    #[test]
    fn dynamic_argument_permission_filters_value_suggestions() {
        let root_permission =
            PermissionKey::parse("minecraft.command.gamemode").expect("permission key parses");
        let mut catalog = PermissionCatalog::new();
        let root = commands::gamemode::command()
            .resolve_subcommand_permissions(&root_permission, &mut catalog)
            .expect("gamemode permissions resolve");
        let graph = graph_with_root(root);

        assert!(graph.suggest("gamemode ", &player_context()).is_none());

        let survival = player_context_with(
            PermissionKey::parse("minecraft.command.gamemode.survival")
                .expect("permission key parses"),
        );
        let result = graph
            .suggest("gamemode ", &survival)
            .expect("survival suggestion");

        assert_eq!(suggestion_texts(&result), vec!["survival".to_owned()]);

        let root = player_context_with(root_permission);
        let result = graph.suggest("gamemode ", &root).expect("root suggestions");

        assert_eq!(
            suggestion_texts(&result),
            vec![
                "survival".to_owned(),
                "creative".to_owned(),
                "adventure".to_owned(),
                "spectator".to_owned()
            ]
        );
    }

    #[test]
    fn denied_dynamic_argument_hides_deeper_suggestions() {
        let root_permission =
            PermissionKey::parse("minecraft.command.gamemode").expect("permission key parses");
        let mut catalog = PermissionCatalog::new();
        let root = commands::gamemode::command()
            .resolve_subcommand_permissions(&root_permission, &mut catalog)
            .expect("gamemode permissions resolve");
        let graph = graph_with_root(root);

        assert!(
            graph
                .suggest("gamemode creative @", &player_context())
                .is_none()
        );

        let creative = player_context_with(
            PermissionKey::parse("minecraft.command.gamemode.creative")
                .expect("permission key parses"),
        );
        let result = graph
            .suggest("gamemode creative @", &creative)
            .expect("target suggestions");

        assert_eq!(
            suggestion_texts(&result),
            vec![
                "@a".to_owned(),
                "@p".to_owned(),
                "@r".to_owned(),
                "@s".to_owned()
            ]
        );
    }

    #[test]
    fn parses_literal_and_integer_argument() {
        let graph = graph_with_root(
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
    fn leading_whitespace_is_not_skipped_at_root() {
        let graph = graph_with_root(literal("list").executes(|_, _| Ok(CommandResult::success())));

        let error = graph
            .parse(" list", &player_context())
            .expect_err("leading whitespace should not be accepted");

        assert_eq!(error.kind(), &CommandParseErrorKind::ExpectedArgument);
        assert_eq!(error.cursor(), 0);
    }

    #[test]
    fn parses_named_arguments_for_executors() {
        let graph =
            graph_with_root(literal("flag").then(
                argument("enabled", BoolParser).executes(|_, _| Ok(CommandResult::success())),
            ));

        let result = graph
            .parse("flag true", &player_context())
            .expect("command parses");

        assert_eq!(result.arguments().get::<bool>("enabled"), Ok(true));
    }

    #[test]
    fn parses_bounded_float_argument() {
        let graph = graph_with_root(
            literal("speed").then(
                argument("value", FloatParser::bounded(Some(0.0), Some(30.0)))
                    .executes(|_, _| Ok(CommandResult::success())),
            ),
        );

        let result = graph
            .parse("speed 1.5", &player_context())
            .expect("command parses");

        assert_eq!(result.arguments().get::<f32>("value"), Ok(1.5));

        let error = graph
            .parse("speed 31.0", &player_context())
            .expect_err("out-of-range float should fail");
        assert_eq!(
            error.kind(),
            &CommandParseErrorKind::FloatTooHigh {
                value: 31.0,
                max: 30.0
            }
        );
    }

    #[test]
    fn quoted_string_argument_keeps_spaces() {
        let graph = graph_with_root(
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
    fn redirect_node_captures_remaining_command_tail() {
        let graph = graph_with_root(literal("execute").then(
            literal("run").redirects(CommandRedirectTarget::All, |_, _| {
                Ok(CommandResult::success())
            }),
        ));

        let result = graph
            .parse("execute run say hello", &player_context())
            .expect("redirect parses");

        assert_eq!(result.path(), ["execute", "run"]);
        let ParsedCommandAction::Redirect(redirect) = &result.action else {
            panic!("expected redirect action");
        };
        assert_eq!(redirect.target, CommandRedirectTarget::All);
        assert_eq!(redirect.command, "say hello");
        assert_eq!(redirect.current_root, "execute");
    }

    #[test]
    fn trailing_data_is_distinct_from_incomplete_command() {
        let graph = graph_with_root(literal("list").executes(|_, _| Ok(CommandResult::success())));

        let error = graph
            .parse("list extra", &player_context())
            .expect_err("extra input should fail");

        assert_eq!(error.kind(), &CommandParseErrorKind::TrailingData);
        assert_eq!(error.cursor(), 5);
    }

    #[test]
    fn branch_errors_prefer_farthest_cursor() {
        let graph = graph_with_root(
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
        let graph = graph_with_root(
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
    fn usage_hides_unusable_roots() {
        let graph = CommandGraph::new()
            .with_root(literal("open"))
            .expect("open root registers")
            .with_root(
                literal("admin").requires(Requirement::Permission(PermissionExpr::key(
                    PermissionKey::parse("steel.admin").expect("key parses"),
                ))),
            )
            .expect("admin root registers");
        let mut nodes = vec![ProtocolCommandNode::new_root()];
        let mut root_children = Vec::new();

        graph.usage(&mut nodes, &mut root_children, &player_context());

        assert_eq!(
            literal_names(&nodes, &root_children),
            vec!["open".to_owned()]
        );

        let allowed_context =
            player_context_with(PermissionKey::parse("steel.admin").expect("key parses"));
        let mut nodes = vec![ProtocolCommandNode::new_root()];
        let mut root_children = Vec::new();

        graph.usage(&mut nodes, &mut root_children, &allowed_context);

        assert_eq!(
            literal_names(&nodes, &root_children),
            vec!["open".to_owned(), "admin".to_owned()]
        );
    }

    #[test]
    fn usage_marks_permission_nodes_restricted() {
        const RESTRICTED_FLAG: u8 = 32;

        let graph = graph_with_root(literal("admin").requires(Requirement::Permission(
            PermissionExpr::key(PermissionKey::parse("steel.admin").expect("key parses")),
        )));
        let allowed_context =
            player_context_with(PermissionKey::parse("steel.admin").expect("key parses"));
        let mut nodes = vec![ProtocolCommandNode::new_root()];
        let mut root_children = Vec::new();

        graph.usage(&mut nodes, &mut root_children, &allowed_context);

        let admin = usize::try_from(root_children[0]).expect("node index is non-negative");
        assert_ne!(node_flag_byte(&nodes[admin]) & RESTRICTED_FLAG, 0);
    }

    #[test]
    fn root_suggestions_include_partial_literals() {
        let graph = graph_with_root(literal("list"));

        let result = graph
            .suggest("li", &player_context())
            .expect("root suggestion");

        assert_eq!(suggestion_texts(&result), vec!["list".to_owned()]);
        assert_eq!(result.start, 0);
        assert_eq!(result.length, 2);
    }

    #[test]
    fn slash_root_suggestions_keep_client_offset() {
        let graph = graph_with_root(literal("list"));

        let result = graph
            .suggest("/li", &player_context())
            .expect("root suggestion");

        assert_eq!(suggestion_texts(&result), vec!["list".to_owned()]);
        assert_eq!(result.start, 1);
        assert_eq!(result.length, 2);
    }

    #[test]
    fn leading_whitespace_does_not_suggest_roots() {
        let graph = graph_with_root(literal("list"));

        assert!(graph.suggest(" l", &player_context()).is_none());
    }

    #[test]
    fn child_literal_suggestions_use_child_range() {
        let graph = graph_with_root(literal("list").then(literal("uuids")));

        let result = graph
            .suggest("list u", &player_context())
            .expect("child suggestion");

        assert_eq!(suggestion_texts(&result), vec!["uuids".to_owned()]);
        assert_eq!(result.start, 5);
        assert_eq!(result.length, 1);
    }

    #[test]
    fn trailing_space_suggests_children() {
        let graph = graph_with_root(literal("list").then(literal("uuids")));

        let result = graph
            .suggest("list ", &player_context())
            .expect("child suggestion");

        assert_eq!(suggestion_texts(&result), vec!["uuids".to_owned()]);
        assert_eq!(result.start, 5);
        assert_eq!(result.length, 0);
    }

    #[test]
    fn trailing_space_after_leaf_has_no_stale_suggestion() {
        let graph = graph_with_root(literal("list").then(literal("uuids")));

        assert!(graph.suggest("list uuids ", &player_context()).is_none());
    }

    #[test]
    fn bool_parser_suggests_values() {
        let graph = graph_with_root(literal("flag").then(argument("enabled", BoolParser)));

        let result = graph
            .suggest("flag f", &player_context())
            .expect("bool suggestion");

        assert_eq!(suggestion_texts(&result), vec!["false".to_owned()]);
        assert_eq!(result.start, 5);
        assert_eq!(result.length, 1);
    }

    fn literal_names(nodes: &[ProtocolCommandNode], indexes: &[i32]) -> Vec<String> {
        indexes
            .iter()
            .filter_map(|index| {
                let index = usize::try_from(*index).ok()?;
                match &nodes[index] {
                    ProtocolCommandNode::Literal { name, .. } => Some(name.to_string()),
                    ProtocolCommandNode::Root { .. } | ProtocolCommandNode::Argument { .. } => None,
                }
            })
            .collect()
    }

    fn node_flag_byte(node: &ProtocolCommandNode) -> u8 {
        let mut bytes = Vec::new();
        node.write(&mut bytes).expect("command node writes");
        bytes[0]
    }

    #[test]
    fn duplicate_literal_roots_merge_child_branches() {
        let graph = CommandGraph::new()
            .with_root(literal("root").then(literal("one")))
            .expect("first root branch registers")
            .with_root(literal("root").then(literal("two")))
            .expect("second root branch registers");

        let root_result = graph
            .suggest("ro", &player_context())
            .expect("root suggestion");
        assert_eq!(suggestion_texts(&root_result), vec!["root".to_owned()]);

        let child_result = graph
            .suggest("root ", &player_context())
            .expect("child suggestions");
        assert_eq!(
            suggestion_texts(&child_result),
            vec!["one".to_owned(), "two".to_owned()]
        );
    }

    #[test]
    fn redirect_to_all_suggests_dispatcher_roots() {
        let graph = CommandGraph::new()
            .with_root(literal("give"))
            .expect("give root registers")
            .with_root(literal("execute").then(
                literal("run").redirects(CommandRedirectTarget::All, |_, _| {
                    Ok(CommandResult::success())
                }),
            ))
            .expect("execute root registers");

        let result = graph
            .suggest("execute run gi", &player_context())
            .expect("redirect target suggestion");

        assert_eq!(suggestion_texts(&result), vec!["give".to_owned()]);
        assert_eq!(result.start, 12);
        assert_eq!(result.length, 2);
    }

    #[test]
    fn redirect_to_current_suggests_current_root_children() {
        let graph = graph_with_root(
            literal("execute")
                .then(
                    literal("anchored").then(
                        argument("anchor", AnchorParser)
                            .redirects(CommandRedirectTarget::Current, |_, _| {
                                Ok(CommandResult::success())
                            }),
                    ),
                )
                .then(
                    literal("run").redirects(CommandRedirectTarget::All, |_, _| {
                        Ok(CommandResult::success())
                    }),
                ),
        );

        let result = graph
            .suggest("execute anchored eyes ru", &player_context())
            .expect("current redirect suggestion");

        assert_eq!(suggestion_texts(&result), vec!["run".to_owned()]);
        assert_eq!(result.start, 22);
        assert_eq!(result.length, 2);
    }
}
