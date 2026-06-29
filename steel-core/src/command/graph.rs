//! Dynamic command graph, parsers, and structured parse results.

use std::{error::Error, fmt, sync::Arc};

use steel_protocol::packets::game::{CommandNode as ProtocolCommandNode, SuggestionEntry};

use crate::command::{
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
    BiomeArgumentValue, BlockPredicateArgumentValue, CommandPermissionArgument, FromParsedArgument,
    IntRangeArgumentValue, ItemPredicateArgumentValue, ItemPredicateCondition,
    ItemPredicateMatchError, ItemPredicateTarget, ItemPredicateTerm, ItemSlotRangeArgumentValue,
    ParsedArgument, ParsedArgumentError, ParsedArguments, PermissionTarget,
    ScoreHolderArgumentValue, ScoreboardObjectiveName, StructureArgumentValue,
};
pub use builder::{CommandNodeBuilder, argument, literal};
use node::{CommandNode, CommandNodeKind, collect_ambiguities, merge_or_push_node};
pub use primitive_parsers::{
    AnchorParser, BoolParser, CommandArgumentClientParser, CommandArgumentParser, DoubleParser,
    FloatParser, IntegerParser, LongParser, StringParser,
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
    /// A long integer argument was invalid.
    InvalidLong(String),
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
    /// A long integer argument was below its minimum.
    LongTooLow {
        /// Parsed value.
        value: i64,
        /// Minimum accepted value.
        min: i64,
    },
    /// A long integer argument was above its maximum.
    LongTooHigh {
        /// Parsed value.
        value: i64,
        /// Maximum accepted value.
        max: i64,
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
    /// A double argument was invalid.
    InvalidDouble(String),
    /// A double argument was below its minimum.
    DoubleTooLow {
        /// Parsed value.
        value: f64,
        /// Minimum accepted value.
        min: f64,
    },
    /// A double argument was above its maximum.
    DoubleTooHigh {
        /// Parsed value.
        value: f64,
        /// Maximum accepted value.
        max: f64,
    },
    /// A game mode argument was invalid.
    InvalidGameMode(String),
    /// A player argument was invalid.
    InvalidPlayer(String),
    /// An entity argument was invalid.
    InvalidEntity(String),
    /// Entity selector syntax is not allowed for this source.
    EntitySelectorsNotAllowed,
    /// Entity selector syntax was malformed.
    InvalidEntitySelector(String),
    /// Entity selector option is recognized but needs a missing runtime foundation.
    UnsupportedEntitySelectorOption(String),
    /// An entity type argument was invalid.
    InvalidEntityType(String),
    /// An item argument was invalid.
    InvalidItem(String),
    /// An item stack argument was invalid.
    InvalidItemStack(String),
    /// An item slot range argument was invalid.
    InvalidItemSlot(String),
    /// An item predicate argument was invalid.
    InvalidItemPredicate(String),
    /// An enchantment argument was invalid.
    InvalidEnchantment(String),
    /// A biome argument was invalid.
    InvalidBiome(String),
    /// A block predicate argument was invalid.
    InvalidBlockPredicate(String),
    /// An NBT path argument was invalid.
    InvalidNbtPath(String),
    /// A structure argument was invalid.
    InvalidStructure(String),
    /// A domain argument was invalid.
    InvalidDomain(String),
    /// A world argument was invalid.
    InvalidWorld(String),
    /// An identifier argument was invalid.
    InvalidIdentifier(String),
    /// A 3D vector argument was invalid.
    InvalidVec3(String),
    /// A block position argument was invalid.
    InvalidBlockPos(String),
    /// A heightmap argument was invalid.
    InvalidHeightmap(String),
    /// A rotation argument was invalid.
    InvalidRotation(String),
    /// A swizzle argument was invalid.
    InvalidSwizzle(String),
    /// A text component argument was invalid.
    InvalidComponent(String),
    /// An integer range argument was invalid.
    InvalidIntegerRange(String),
    /// An integer range had a minimum larger than its maximum.
    SwappedIntegerRange,
    /// A time argument was invalid.
    InvalidTime(String),
    /// A permission key argument was invalid.
    InvalidPermissionKey(String),
    /// A permission rule expression argument was invalid.
    InvalidPermissionExpression(String),
    /// A permission metadata expression argument was invalid.
    InvalidPermissionMetadataExpression(String),
    /// A permission metadata key argument was invalid.
    InvalidPermissionMetadataKey(String),
    /// A permission group argument was invalid.
    InvalidPermissionGroup(String),
    /// A parser required live command context that was not available.
    MissingCommandContext(&'static str),
    /// An argument parser returned success without advancing the reader.
    ArgumentParserDidNotConsumeInput {
        /// Argument node name.
        argument: String,
        /// Parser value type.
        parsed_type: &'static str,
    },
}

impl CommandParseErrorKind {
    const fn precedence(&self) -> u8 {
        match self {
            Self::TrailingData => 7,
            Self::InvalidBool(_)
            | Self::InvalidAnchor(_)
            | Self::InvalidInteger(_)
            | Self::InvalidLong(_)
            | Self::IntegerTooLow { .. }
            | Self::IntegerTooHigh { .. }
            | Self::LongTooLow { .. }
            | Self::LongTooHigh { .. }
            | Self::InvalidFloat(_)
            | Self::FloatTooLow { .. }
            | Self::FloatTooHigh { .. }
            | Self::InvalidDouble(_)
            | Self::DoubleTooLow { .. }
            | Self::DoubleTooHigh { .. }
            | Self::InvalidGameMode(_)
            | Self::InvalidPlayer(_)
            | Self::InvalidEntity(_)
            | Self::EntitySelectorsNotAllowed
            | Self::InvalidEntitySelector(_)
            | Self::UnsupportedEntitySelectorOption(_)
            | Self::InvalidEntityType(_)
            | Self::InvalidItem(_)
            | Self::InvalidItemStack(_)
            | Self::InvalidItemSlot(_)
            | Self::InvalidItemPredicate(_)
            | Self::InvalidEnchantment(_)
            | Self::InvalidBiome(_)
            | Self::InvalidBlockPredicate(_)
            | Self::InvalidNbtPath(_)
            | Self::InvalidStructure(_)
            | Self::InvalidDomain(_)
            | Self::InvalidWorld(_)
            | Self::InvalidIdentifier(_)
            | Self::InvalidVec3(_)
            | Self::InvalidBlockPos(_)
            | Self::InvalidHeightmap(_)
            | Self::InvalidRotation(_)
            | Self::InvalidSwizzle(_)
            | Self::InvalidComponent(_)
            | Self::InvalidIntegerRange(_)
            | Self::SwappedIntegerRange
            | Self::InvalidTime(_)
            | Self::InvalidPermissionKey(_)
            | Self::InvalidPermissionExpression(_)
            | Self::InvalidPermissionMetadataExpression(_)
            | Self::InvalidPermissionMetadataKey(_)
            | Self::InvalidPermissionGroup(_)
            | Self::MissingCommandContext(_)
            | Self::ArgumentParserDidNotConsumeInput { .. }
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
    /// A derived subcommand permission was requested on a permission-path passthrough node.
    DerivedPermissionRequiresPermissionPath {
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
    /// A terminal argument parser was registered with unreachable tail nodes.
    TerminalArgumentMustBeLeaf {
        /// Argument node name.
        name: String,
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
            Self::DerivedPermissionRequiresPermissionPath { name } => {
                write!(
                    f,
                    "derived subcommand permission requires a permission path segment, got passthrough node '{name}'"
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
            Self::TerminalArgumentMustBeLeaf { name } => {
                write!(
                    f,
                    "argument '{name}' is terminal and cannot have child nodes or redirects"
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
    /// Integer result returned by the command.
    pub result: i32,
}

impl CommandResult {
    /// Creates a successful command result.
    #[must_use]
    pub const fn success() -> Self {
        Self::from_return_value(1)
    }

    /// Creates a direct command result with the same command return value and callback result.
    #[must_use]
    pub const fn from_return_value(value: i32) -> Self {
        Self {
            success_count: value,
            result: value,
        }
    }

    /// Creates a command result where the result value is the success count.
    #[must_use]
    pub const fn from_success_count(success_count: i32) -> Self {
        Self::from_return_value(success_count)
    }

    /// Creates a command result from a `usize` success count, saturating at `i32::MAX`.
    #[must_use]
    pub fn from_usize_success_count(success_count: usize) -> Self {
        Self::from_success_count(i32::try_from(success_count).map_or(i32::MAX, |count| count))
    }

    /// Creates a command result with separate success-count and result values.
    #[must_use]
    pub const fn with_result(success_count: i32, result: i32) -> Self {
        Self {
            success_count,
            result,
        }
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

/// Example-based ambiguity between sibling command nodes.
///
/// This mirrors Brigadier's ambiguity diagnostics: parsers provide
/// representative examples, and validation checks whether a sibling can also
/// parse those examples. It is intentionally diagnostic, not a full grammar
/// proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandGraphAmbiguity {
    /// Path to the parent whose children are ambiguous.
    pub parent_path: Vec<String>,
    /// Child node that provided the matching examples.
    pub child: String,
    /// Sibling node that also accepts those examples.
    pub sibling: String,
    /// Example inputs accepted by both nodes.
    pub inputs: Vec<String>,
}

/// Diagnostic command graph validation result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandGraphValidation {
    ambiguities: Vec<CommandGraphAmbiguity>,
}

impl CommandGraphValidation {
    /// Returns example-based sibling ambiguities.
    #[must_use]
    pub fn ambiguities(&self) -> &[CommandGraphAmbiguity] {
        &self.ambiguities
    }

    /// Returns true when the graph has no known validation diagnostics.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ambiguities.is_empty()
    }
}

type CommandExecutor = Arc<
    dyn Fn(&mut CommandContext, &ParsedArguments) -> Result<CommandResult, CommandError>
        + Send
        + Sync,
>;
type CommandForkExecutor = Arc<
    dyn Fn(&mut CommandContext, &ParsedArguments) -> Result<Vec<CommandContext>, CommandError>
        + Send
        + Sync,
>;
type UnresolvedDynamicPermissionResolver = Arc<
    dyn Fn(&PermissionKey, &ParsedArguments) -> Result<PermissionKey, DynamicPermissionError>
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
                Ok(base_permission.child(&segment)?)
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
                let key = resolver(&base_permission, arguments)?;
                Ok(PermissionExpr::scoped_key(root_permission.clone(), key))
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
    modifier: ParsedRedirectModifier,
}

#[derive(Clone)]
enum ParsedRedirectModifier {
    Single(CommandExecutor),
    Fork(CommandForkExecutor),
}

pub(crate) enum CommandExecutionStep {
    Complete(CommandResult),
    Redirect {
        command: String,
        contexts: Vec<CommandContext>,
        forked: bool,
    },
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
        match self.execute_step(context)? {
            CommandExecutionStep::Complete(result) => Ok(result),
            CommandExecutionStep::Redirect { command, .. } => Err(CommandError::failure(format!(
                "Command redirect target '{command}' is unavailable"
            ))),
        }
    }

    /// Executes this parsed command or returns the next redirected command tail.
    ///
    /// # Errors
    ///
    /// Returns an error from the matched executor.
    pub(crate) fn execute_step(
        &self,
        context: &mut CommandContext,
    ) -> Result<CommandExecutionStep, CommandError> {
        self.check_dynamic_permissions(context)?;
        match &self.action {
            ParsedCommandAction::Execute(executor) => {
                executor(context, &self.arguments).map(CommandExecutionStep::Complete)
            }
            ParsedCommandAction::Redirect(redirect) => {
                let contexts = match &redirect.modifier {
                    ParsedRedirectModifier::Single(executor) => {
                        let mut redirect_context = context.clone();
                        executor(&mut redirect_context, &self.arguments)?;
                        vec![redirect_context]
                    }
                    ParsedRedirectModifier::Fork(executor) => {
                        let mut redirect_context = context.clone();
                        executor(&mut redirect_context, &self.arguments)?
                    }
                };
                let command = match redirect.target {
                    CommandRedirectTarget::Current => {
                        format!("{} {}", redirect.current_root, redirect.command)
                    }
                    CommandRedirectTarget::All => redirect.command.clone(),
                };
                Ok(CommandExecutionStep::Redirect {
                    command,
                    contexts,
                    forked: matches!(redirect.modifier, ParsedRedirectModifier::Fork(_)),
                })
            }
        }
    }

    pub(crate) fn invokes_result_callback_on_error(&self) -> bool {
        matches!(self.action, ParsedCommandAction::Execute(_))
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

    /// Returns diagnostic validation information for this graph.
    #[must_use]
    pub fn validate(&self) -> CommandGraphValidation {
        let mut validation = CommandGraphValidation::default();
        collect_ambiguities(&self.roots, &mut Vec::new(), &mut validation.ambiguities);
        validation
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
mod tests;
