//! Dynamic command graph, parsers, and structured parse results.

use std::{fmt, sync::Arc};

use glam::DVec3;
use steel_protocol::packets::game::{
    ArgumentStringTypeBehavior, ArgumentType, CommandNode as ProtocolCommandNode, CommandNodeInfo,
    SuggestionEntry, SuggestionType,
};
use steel_registry::{
    enchantment::EnchantmentRef, entity_type::EntityTypeRef, items::ItemRef,
    structure::StructureRef,
};
use steel_utils::types::GameType;
use steel_utils::{BlockPos, Identifier};
use text_components::TextComponent;

use crate::command::{
    context::{CommandContext, EntityAnchor},
    error::CommandError,
    reader::{CommandReader, StringMode},
    requirement::{CommandInputContext, Requirement, RequirementContext},
};
use crate::entity::LivingEntity;
use crate::permission::{PermissionExpr, PermissionKey, PermissionKeyError};
use crate::player::Player;
use crate::world::World;

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

/// A parsed command argument value.
#[derive(Clone)]
pub enum ParsedArgument {
    /// Boolean argument.
    Bool(bool),
    /// Entity anchor argument.
    Anchor(EntityAnchor),
    /// 32-bit signed integer argument.
    I32(i32),
    /// 32-bit floating-point argument.
    F32(f32),
    /// String-like argument.
    String(String),
    /// Game mode argument.
    GameMode(GameType),
    /// Player target argument.
    Players(Vec<Arc<Player>>),
    /// Living entity target argument.
    Entities(Vec<Arc<dyn LivingEntity + Send + Sync>>),
    /// Entity type argument.
    EntityType(EntityTypeRef),
    /// Item argument.
    Item(ItemRef),
    /// Enchantment argument.
    Enchantment(EnchantmentRef),
    /// Structure or structure tag argument.
    Structure(StructureArgumentValue),
    /// Loaded world argument.
    World(Arc<World>),
    /// 3D vector argument.
    Vec3(DVec3),
    /// Block position argument.
    BlockPos(BlockPos),
    /// Rotation argument.
    Rotation((f32, f32)),
    /// Text component argument.
    Component(Box<TextComponent>),
}

/// Structure command argument value: either one structure or a structure tag.
#[derive(Clone, Debug)]
pub enum StructureArgumentValue {
    /// A single structure key.
    Structure(StructureRef),
    /// A structure tag and its resolved entries.
    Tag {
        /// Tag key without the leading `#`.
        key: Identifier,
        /// Structures in the tag.
        structures: Vec<StructureRef>,
    },
}

impl StructureArgumentValue {
    /// Structure keys to scan.
    #[must_use]
    pub fn structure_keys(&self) -> Vec<Identifier> {
        match self {
            Self::Structure(structure) => vec![structure.key.clone()],
            Self::Tag { structures, .. } => structures
                .iter()
                .map(|structure| structure.key.clone())
                .collect(),
        }
    }

    /// Printable command target name.
    #[must_use]
    pub fn printable_name(&self, found_structure: &Identifier) -> String {
        match self {
            Self::Structure(structure) => structure.key.to_string(),
            Self::Tag { key, .. } => format!("#{key} ({found_structure})"),
        }
    }

    /// Printable command target without resolved found entry.
    #[must_use]
    pub fn query_name(&self) -> String {
        match self {
            Self::Structure(structure) => structure.key.to_string(),
            Self::Tag { key, .. } => format!("#{key}"),
        }
    }
}

impl fmt::Debug for ParsedArgument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(value) => f.debug_tuple("Bool").field(value).finish(),
            Self::Anchor(value) => f.debug_tuple("Anchor").field(value).finish(),
            Self::I32(value) => f.debug_tuple("I32").field(value).finish(),
            Self::F32(value) => f.debug_tuple("F32").field(value).finish(),
            Self::String(value) => f.debug_tuple("String").field(value).finish(),
            Self::GameMode(value) => f.debug_tuple("GameMode").field(value).finish(),
            Self::Players(value) => f
                .debug_struct("Players")
                .field("count", &value.len())
                .finish(),
            Self::Entities(value) => f
                .debug_struct("Entities")
                .field("count", &value.len())
                .finish(),
            Self::EntityType(value) => f.debug_tuple("EntityType").field(&value.key).finish(),
            Self::Item(value) => f.debug_tuple("Item").field(&value.key).finish(),
            Self::Enchantment(value) => f.debug_tuple("Enchantment").field(&value.key).finish(),
            Self::Structure(value) => f.debug_tuple("Structure").field(value).finish(),
            Self::World(value) => f.debug_tuple("World").field(&value.key).finish(),
            Self::Vec3(value) => f.debug_tuple("Vec3").field(value).finish(),
            Self::BlockPos(value) => f.debug_tuple("BlockPos").field(value).finish(),
            Self::Rotation(value) => f.debug_tuple("Rotation").field(value).finish(),
            Self::Component(_) => f.debug_tuple("Component").finish(),
        }
    }
}

/// Typed command argument storage.
#[derive(Clone, Debug, Default)]
pub struct ParsedArguments {
    values: Vec<ParsedArgumentEntry>,
}

#[derive(Clone, Debug)]
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
            Self::Anchor(_) => "anchor",
            Self::I32(_) => "i32",
            Self::F32(_) => "f32",
            Self::String(_) => "string",
            Self::GameMode(_) => "gamemode",
            Self::Players(_) => "players",
            Self::Entities(_) => "entities",
            Self::EntityType(_) => "entity_type",
            Self::Item(_) => "item",
            Self::Enchantment(_) => "enchantment",
            Self::Structure(_) => "structure",
            Self::World(_) => "world",
            Self::Vec3(_) => "vec3",
            Self::BlockPos(_) => "block_pos",
            Self::Rotation(_) => "rotation",
            Self::Component(_) => "component",
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

impl FromParsedArgument for EntityAnchor {
    const TYPE_NAME: &'static str = "anchor";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Anchor(value) = value else {
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

impl FromParsedArgument for f32 {
    const TYPE_NAME: &'static str = "f32";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::F32(value) = value else {
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

impl FromParsedArgument for GameType {
    const TYPE_NAME: &'static str = "gamemode";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::GameMode(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for Vec<Arc<Player>> {
    const TYPE_NAME: &'static str = "players";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Players(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for Vec<Arc<dyn LivingEntity + Send + Sync>> {
    const TYPE_NAME: &'static str = "entities";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Entities(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for EntityTypeRef {
    const TYPE_NAME: &'static str = "entity_type";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::EntityType(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for ItemRef {
    const TYPE_NAME: &'static str = "item";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Item(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for EnchantmentRef {
    const TYPE_NAME: &'static str = "enchantment";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Enchantment(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for StructureArgumentValue {
    const TYPE_NAME: &'static str = "structure";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Structure(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for Arc<World> {
    const TYPE_NAME: &'static str = "world";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::World(value) = value else {
            return None;
        };
        Some(Arc::clone(value))
    }
}

impl FromParsedArgument for DVec3 {
    const TYPE_NAME: &'static str = "vec3";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Vec3(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for BlockPos {
    const TYPE_NAME: &'static str = "block_pos";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::BlockPos(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for (f32, f32) {
    const TYPE_NAME: &'static str = "rotation";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Rotation(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for TextComponent {
    const TYPE_NAME: &'static str = "component";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Component(value) = value else {
            return None;
        };
        Some(value.as_ref().clone())
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

/// Target for redirecting graph execution after a node action runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandRedirectTarget {
    /// Redirects back to the current root command.
    Current,
    /// Redirects to the command dispatcher root.
    All,
}

#[derive(Clone)]
struct CommandRedirect {
    target: CommandRedirectTarget,
    executor: CommandExecutor,
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
        self.execute_with_dispatcher(context, |command, _| {
            Err(CommandError::CommandFailed(Box::new(TextComponent::plain(
                format!("Command redirect target '{command}' is unavailable"),
            ))))
        })
    }

    /// Executes this parsed command, using `dispatch` for redirected command tails.
    ///
    /// # Errors
    ///
    /// Returns an error from either this command's executor or the redirected command.
    pub fn execute_with_dispatcher(
        &self,
        context: &mut CommandContext,
        mut dispatch: impl FnMut(&str, &mut CommandContext) -> Result<CommandResult, CommandError>,
    ) -> Result<CommandResult, CommandError> {
        match &self.action {
            ParsedCommandAction::Execute(executor) => executor(context, &self.arguments),
            ParsedCommandAction::Redirect(redirect) => {
                (redirect.executor)(context, &self.arguments)?;
                match redirect.target {
                    CommandRedirectTarget::Current => {
                        let command = format!("{} {}", redirect.current_root, redirect.command);
                        dispatch(&command, context)
                    }
                    CommandRedirectTarget::All => dispatch(&redirect.command, context),
                }
            }
        }
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
        self.register_root(root);
        self
    }

    /// Adds a root command node.
    pub fn register_root(&mut self, root: CommandNodeBuilder) {
        merge_or_push_node(&mut self.roots, root.build());
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
        let mut reader = CommandReader::with_offset(input_without_slash, cursor_offset);
        reader.skip_whitespace();

        suggest_children(
            &reader,
            &self.roots,
            ParsedArguments::default(),
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

    if node.redirect.is_some() {
        return node.redirectable(input, arguments, path, reader.remaining().to_owned());
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
    roots: &[CommandNode],
    current_root_children: Option<&[CommandNode]>,
) -> Option<SuggestionResult> {
    let mut best_result = None;

    for child in children {
        if !child.requirement.allows(context) {
            continue;
        }

        let child_root_children = current_root_children.or_else(|| {
            matches!(&child.kind, CommandNodeKind::Literal(_)).then_some(child.children.as_slice())
        });

        if let Some(result) = child.suggest(
            reader,
            arguments.clone(),
            context,
            roots,
            child_root_children,
        ) {
            keep_best_suggestion(&mut best_result, result);
        }
    }

    best_result
}

fn merge_or_push_node(nodes: &mut Vec<CommandNode>, node: CommandNode) {
    let Some(existing) = nodes
        .iter_mut()
        .find(|existing| existing.can_merge_with(&node))
    else {
        nodes.push(node);
        return;
    };

    existing.merge(node);
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
    redirect: Option<CommandRedirect>,
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

    /// Adds a permission requirement to this node.
    ///
    /// # Errors
    ///
    /// Returns an error when `permission` is not a valid permission key.
    pub fn requires_permission(
        self,
        permission: impl Into<String>,
    ) -> Result<Self, PermissionKeyError> {
        Ok(self.requires_permission_expr(PermissionExpr::key(PermissionKey::parse(permission)?)))
    }

    /// Adds a permission expression requirement to this node.
    #[must_use]
    pub fn requires_permission_expr(self, permission: PermissionExpr) -> Self {
        self.requires(Requirement::Permission(permission))
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

    /// Redirects from this node after running `executor`.
    #[must_use]
    pub fn redirects(
        mut self,
        target: CommandRedirectTarget,
        executor: impl Fn(&mut CommandContext, &ParsedArguments) -> Result<CommandResult, CommandError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.redirect = Some(CommandRedirect {
            target,
            executor: Arc::new(executor),
        });
        self
    }

    fn build(self) -> CommandNode {
        CommandNode {
            kind: self.kind,
            requirement: self.requirement,
            children: self.children.into_iter().map(Self::build).collect(),
            executor: self.executor,
            redirect: self.redirect,
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
        redirect: None,
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
        redirect: None,
    }
}

struct CommandNode {
    kind: CommandNodeKind,
    requirement: Requirement,
    children: Vec<CommandNode>,
    executor: Option<CommandExecutor>,
    redirect: Option<CommandRedirect>,
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
            action: ParsedCommandAction::Execute(Arc::clone(self.executor.as_ref()?)),
        })
    }

    fn redirectable(
        &self,
        input: &str,
        arguments: ParsedArguments,
        path: Vec<String>,
        command: String,
    ) -> Result<ParseResults, CommandParseError> {
        let Some(redirect) = &self.redirect else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::TrailingData,
                input.len(),
            ));
        };
        let Some(current_root) = path.first().cloned() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::IncompleteCommand,
                input.len(),
            ));
        };

        Ok(ParseResults {
            input: input.to_owned(),
            arguments,
            path,
            action: ParsedCommandAction::Redirect(ParsedRedirect {
                target: redirect.target,
                current_root,
                command,
                executor: Arc::clone(&redirect.executor),
            }),
        })
    }

    fn suggest(
        &self,
        reader: &CommandReader<'_>,
        mut arguments: ParsedArguments,
        context: &dyn CommandInputContext,
        roots: &[CommandNode],
        current_root_children: Option<&[CommandNode]>,
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
        if let Some(deeper) = self.suggest_after_successful_parse(
            &mut parsed_reader,
            arguments,
            context,
            roots,
            current_root_children,
        ) {
            keep_best_suggestion(&mut result, deeper);
        }
        result
    }

    fn suggest_after_successful_parse(
        &self,
        reader: &mut CommandReader<'_>,
        arguments: ParsedArguments,
        context: &dyn CommandInputContext,
        roots: &[CommandNode],
        current_root_children: Option<&[CommandNode]>,
    ) -> Option<SuggestionResult> {
        if !reader.can_read() {
            return None;
        }

        if !reader.peek().is_some_and(char::is_whitespace) {
            return None;
        }

        reader.skip_whitespace();
        if let Some(redirect) = &self.redirect {
            return match redirect.target {
                CommandRedirectTarget::Current => current_root_children.and_then(|children| {
                    suggest_children(
                        reader,
                        children,
                        ParsedArguments::default(),
                        context,
                        roots,
                        current_root_children,
                    )
                }),
                CommandRedirectTarget::All => suggest_children(
                    reader,
                    roots,
                    ParsedArguments::default(),
                    context,
                    roots,
                    None,
                ),
            };
        }

        suggest_children(
            reader,
            &self.children,
            arguments,
            context,
            roots,
            current_root_children,
        )
    }

    fn usage(
        &self,
        buffer: &mut Vec<ProtocolCommandNode>,
        siblings: &mut Vec<i32>,
        context: &dyn RequirementContext,
        current_root_index: Option<i32>,
    ) {
        if !self.requirement.allows(context) {
            return;
        }

        let node_index = buffer.len();
        buffer.push(ProtocolCommandNode::new_root());
        siblings.push(node_index as i32);
        let current_root_index = current_root_index.unwrap_or(node_index as i32);

        let mut children = Vec::new();
        if self.redirect.is_none() {
            for child in &self.children {
                child.usage(buffer, &mut children, context, Some(current_root_index));
            }
        }

        let mut info = self.redirect.as_ref().map_or_else(
            || CommandNodeInfo::new(children),
            |redirect| {
                CommandNodeInfo::new_redirect(match redirect.target {
                    CommandRedirectTarget::Current => current_root_index,
                    CommandRedirectTarget::All => 0,
                })
            },
        );
        if self.redirect.is_none() && self.executor.is_some() {
            info = info.chain(CommandNodeInfo::new_executable());
        }

        buffer[node_index] = match &self.kind {
            CommandNodeKind::Literal(name) => ProtocolCommandNode::new_literal(info, name.clone()),
            CommandNodeKind::Argument { name, parser } => {
                ProtocolCommandNode::new_argument(info, name.clone(), parser.usage())
            }
        };
    }

    fn can_merge_with(&self, other: &Self) -> bool {
        self.requirement == other.requirement
            && self.redirect.is_none()
            && other.redirect.is_none()
            && !(self.executor.is_some() && other.executor.is_some())
            && matches!(
                (&self.kind, &other.kind),
                (CommandNodeKind::Literal(left), CommandNodeKind::Literal(right)) if left == right
            )
    }

    fn merge(&mut self, other: Self) {
        if self.executor.is_none() {
            self.executor = other.executor;
        }

        for child in other.children {
            merge_or_push_node(&mut self.children, child);
        }
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
            AnchorParser, BoolParser, CommandGraph, CommandParseErrorKind, CommandRedirectTarget,
            CommandResult, FloatParser, IntegerParser, ParsedCommandAction, StringParser,
            SuggestionResult, argument, literal,
        },
        reader::StringMode,
        requirement::{
            CommandInputContext, CommandSourceKind, PermissionExpr, PermissionKey, Requirement,
            RequirementContext,
        },
    };
    use crate::permission::{PermissionEntry, PermissionKeyError, PermissionSet};
    use steel_protocol::packets::game::CommandNode as ProtocolCommandNode;

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
        TestContext {
            source_kind: CommandSourceKind::Player,
            permissions: PermissionSet::from_entries([PermissionEntry::allow(permission)]),
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
    fn parses_bounded_float_argument() {
        let graph = CommandGraph::new().with_root(
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
    fn redirect_node_captures_remaining_command_tail() {
        let graph = CommandGraph::new().with_root(literal("execute").then(
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
    fn permission_builder_rejects_invalid_keys() {
        let error = literal("admin").requires_permission("steel.*.admin").err();

        assert_eq!(error, Some(PermissionKeyError::WildcardNotFinal));
    }

    #[test]
    fn usage_hides_unusable_roots() {
        let graph =
            CommandGraph::new()
                .with_root(literal("open"))
                .with_root(literal("admin").requires(Requirement::Permission(
                    PermissionExpr::key(PermissionKey::parse("steel.admin").expect("key parses")),
                )));
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

    #[test]
    fn duplicate_literal_roots_merge_child_branches() {
        let graph = CommandGraph::new()
            .with_root(literal("root").then(literal("one")))
            .with_root(literal("root").then(literal("two")));

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
        let graph =
            CommandGraph::new()
                .with_root(literal("give"))
                .with_root(literal("execute").then(
                    literal("run").redirects(CommandRedirectTarget::All, |_, _| {
                        Ok(CommandResult::success())
                    }),
                ));

        let result = graph
            .suggest("execute run gi", &player_context())
            .expect("redirect target suggestion");

        assert_eq!(suggestion_texts(&result), vec!["give".to_owned()]);
        assert_eq!(result.start, 12);
        assert_eq!(result.length, 2);
    }

    #[test]
    fn redirect_to_current_suggests_current_root_children() {
        let graph = CommandGraph::new().with_root(
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
