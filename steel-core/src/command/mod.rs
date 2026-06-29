//! This module contains everything needed for commands (e.g., parsing, execution, and sender handling).
pub mod commands;
pub mod context;
pub mod error;
mod executor;
pub mod graph;
pub mod parsers;
pub mod reader;
pub mod requirement;
pub mod sender;
pub(crate) mod suggestions;

use steel_protocol::packets::game::{CCommandSuggestions, CCommands, CommandNode, SuggestionEntry};
use steel_utils::translations;
use text_components::TextComponent;

use crate::command::context::{CommandCallbackResult, CommandContext};
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandExecutionStep, CommandGraph, CommandGraphError, CommandNodeBuilder, CommandParseError,
    CommandParseErrorKind, CommandResult, validate_command_node_name,
};
use crate::command::requirement::RequirementContext;
use crate::command::sender::CommandSender;
use crate::permission::{
    PermissionCatalog, PermissionCatalogSource, PermissionContextCatalog, PermissionKey,
    PermissionKeyError, PermissionMetadataCatalog, PermissionSegment,
};
use crate::player::Player;
use crate::server::Server;
use std::{collections::VecDeque, error::Error, fmt, sync::Arc};
use steel_registry::{game_rules::GameRuleValue, vanilla_game_rules::MAX_COMMAND_SEQUENCE_LENGTH};

pub(crate) use executor::CommandQueue;

pub(crate) struct CommandExecutionBudget {
    remaining: usize,
    limit: usize,
}

impl CommandExecutionBudget {
    fn for_context(context: &CommandContext) -> Self {
        Self::new(command_sequence_limit(context))
    }

    fn new(limit: usize) -> Self {
        let limit = limit.max(1);
        Self {
            remaining: limit,
            limit,
        }
    }

    pub(crate) fn consume(&mut self) -> Result<(), CommandError> {
        if self.remaining == 0 {
            return Err(CommandError::failure(format!(
                "Command execution stopped due to command sequence limit ({})",
                self.limit
            )));
        }

        self.remaining -= 1;
        Ok(())
    }
}

fn command_sequence_limit(context: &CommandContext) -> usize {
    match context.world.get_game_rule(&MAX_COMMAND_SEQUENCE_LENGTH) {
        GameRuleValue::Int(value) => value.max(1) as usize,
        GameRuleValue::Bool(_) => default_command_sequence_limit(),
    }
}

fn default_command_sequence_limit() -> usize {
    match MAX_COMMAND_SEQUENCE_LENGTH.default_value {
        GameRuleValue::Int(value) => value.max(1) as usize,
        GameRuleValue::Bool(_) => 1,
    }
}

/// Parses and dispatches commands through the command graph.
#[derive(Clone, Default)]
pub struct CommandDispatcher {
    /// Dynamic command graph.
    graph: CommandGraph,
    permission_catalog: PermissionCatalog,
    permission_metadata_catalog: PermissionMetadataCatalog,
    permission_context_catalog: PermissionContextCatalog,
}

pub(crate) struct CommandRegistration {
    root: CommandNodeBuilder,
    namespace: PermissionSegment,
    permission: CommandPermissionMode,
    aliases: Vec<String>,
}

impl CommandRegistration {
    pub(crate) const fn new(root: CommandNodeBuilder, namespace: PermissionSegment) -> Self {
        Self {
            root,
            namespace,
            permission: CommandPermissionMode::Auto,
            aliases: Vec::new(),
        }
    }

    pub(crate) fn minecraft(root: CommandNodeBuilder) -> Result<Self, CommandRegistrationError> {
        Ok(Self::new(root, PermissionSegment::parse("minecraft")?))
    }

    pub(crate) fn steel(root: CommandNodeBuilder) -> Result<Self, CommandRegistrationError> {
        Ok(Self::new(root, PermissionSegment::parse("steel")?))
    }

    pub(crate) fn public(mut self) -> Self {
        self.permission = CommandPermissionMode::Public;
        self
    }

    pub(crate) fn permission(mut self, permission: PermissionKey) -> Self {
        self.permission = CommandPermissionMode::Override(permission);
        self
    }

    pub(crate) fn permission_base(self, command: &str) -> Result<Self, CommandRegistrationError> {
        let permission = command_permission_key(&self.namespace, command)?;
        Ok(self.permission(permission))
    }

    pub(crate) fn alias(mut self, alias: &str) -> Result<Self, CommandRegistrationError> {
        validate_command_node_name(alias).map_err(|source| {
            CommandGraphError::InvalidLiteralName {
                name: alias.to_owned(),
                source,
            }
        })?;
        self.aliases.push(alias.to_owned());
        Ok(self)
    }

    fn resolved_permission_base(&self) -> Result<Option<PermissionKey>, CommandRegistrationError> {
        match &self.permission {
            CommandPermissionMode::Public => Ok(None),
            CommandPermissionMode::Auto => {
                let command_name = self
                    .root
                    .literal_name()
                    .ok_or(CommandRegistrationError::RootMustBeLiteral)?;
                Ok(Some(command_permission_key(&self.namespace, command_name)?))
            }
            CommandPermissionMode::Override(permission) => Ok(Some(permission.clone())),
        }
    }
}

/// Static registration metadata for one built-in command module.
#[derive(Clone, Copy)]
pub(crate) struct CommandRegistrationSpec {
    namespace: CommandRegistrationNamespace,
    permission: CommandRegistrationSpecPermission,
    aliases: &'static [&'static str],
}

impl CommandRegistrationSpec {
    pub(crate) const fn minecraft() -> Self {
        Self::new(CommandRegistrationNamespace::Minecraft)
    }

    pub(crate) const fn steel() -> Self {
        Self::new(CommandRegistrationNamespace::Steel)
    }

    const fn new(namespace: CommandRegistrationNamespace) -> Self {
        Self {
            namespace,
            permission: CommandRegistrationSpecPermission::Auto,
            aliases: &[],
        }
    }

    pub(crate) const fn public(mut self) -> Self {
        self.permission = CommandRegistrationSpecPermission::Public;
        self
    }

    pub(crate) const fn permission_base(mut self, command: &'static str) -> Self {
        self.permission = CommandRegistrationSpecPermission::PermissionBase(command);
        self
    }

    pub(crate) const fn aliases(mut self, aliases: &'static [&'static str]) -> Self {
        self.aliases = aliases;
        self
    }

    fn register(
        self,
        root: CommandNodeBuilder,
    ) -> Result<CommandRegistration, CommandRegistrationError> {
        let registration = match self.namespace {
            CommandRegistrationNamespace::Minecraft => CommandRegistration::minecraft(root)?,
            CommandRegistrationNamespace::Steel => CommandRegistration::steel(root)?,
        };
        let mut registration = match self.permission {
            CommandRegistrationSpecPermission::Auto => registration,
            CommandRegistrationSpecPermission::Public => registration.public(),
            CommandRegistrationSpecPermission::PermissionBase(command) => {
                registration.permission_base(command)?
            }
        };
        for alias in self.aliases {
            registration = registration.alias(alias)?;
        }
        Ok(registration)
    }
}

#[derive(Clone, Copy)]
enum CommandRegistrationNamespace {
    Minecraft,
    Steel,
}

#[derive(Clone, Copy)]
enum CommandRegistrationSpecPermission {
    Auto,
    Public,
    PermissionBase(&'static str),
}

enum CommandPermissionMode {
    Auto,
    Public,
    Override(PermissionKey),
}

/// Invalid command registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandRegistrationError {
    /// A command root was not a literal node.
    RootMustBeLiteral,
    /// A command or alias produced an invalid permission key.
    InvalidPermissionKey(PermissionKeyError),
    /// The command graph rejected a node registration.
    InvalidGraph(CommandGraphError),
    /// Command graph validation found diagnostics after registration.
    InvalidGraphValidation(crate::command::graph::CommandGraphValidation),
}

impl fmt::Display for CommandRegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RootMustBeLiteral => write!(f, "command root must be a literal node"),
            Self::InvalidPermissionKey(error) => write!(f, "{error}"),
            Self::InvalidGraph(error) => write!(f, "{error}"),
            Self::InvalidGraphValidation(validation) => {
                write!(f, "command graph validation failed")?;
                if let Some(ambiguity) = validation.ambiguities().first() {
                    write!(
                        f,
                        ": ambiguous child '{}' and sibling '{}' under '{}'",
                        ambiguity.child,
                        ambiguity.sibling,
                        ambiguity.parent_path.join(" ")
                    )?;
                }
                Ok(())
            }
        }
    }
}

impl Error for CommandRegistrationError {}

impl From<PermissionKeyError> for CommandRegistrationError {
    fn from(value: PermissionKeyError) -> Self {
        Self::InvalidPermissionKey(value)
    }
}

impl From<CommandGraphError> for CommandRegistrationError {
    fn from(value: CommandGraphError) -> Self {
        Self::InvalidGraph(value)
    }
}

fn command_permission_key(
    namespace: &PermissionSegment,
    command: &str,
) -> Result<PermissionKey, PermissionKeyError> {
    PermissionKey::from_segments([
        namespace.clone(),
        PermissionSegment::parse("command")?,
        PermissionSegment::parse(command)?,
    ])
}

pub(crate) fn minecraft_command_permission_key(
    command: &str,
) -> Result<PermissionKey, PermissionKeyError> {
    command_permission_key(&PermissionSegment::parse("minecraft")?, command)
}

impl CommandDispatcher {
    /// Creates a new command dispatcher with built-in commands.
    ///
    /// # Errors
    ///
    /// Returns an error when a built-in command registration is invalid.
    pub fn new() -> Result<Self, CommandRegistrationError> {
        let mut dispatcher = CommandDispatcher::new_empty();
        for registration in commands::registrations()? {
            dispatcher.register_command(registration)?;
        }
        Ok(dispatcher)
    }

    /// Creates a new command dispatcher with no commands.
    #[must_use]
    pub const fn new_empty() -> Self {
        CommandDispatcher {
            graph: CommandGraph::new(),
            permission_catalog: PermissionCatalog::new(),
            permission_metadata_catalog: PermissionMetadataCatalog::new(),
            permission_context_catalog: PermissionContextCatalog::new(),
        }
    }

    fn register_command(
        &mut self,
        registration: CommandRegistration,
    ) -> Result<(), CommandRegistrationError> {
        let permission_base = registration.resolved_permission_base()?;
        let mut command_catalog = PermissionCatalog::new();
        let root = if let Some(permission_base) = &permission_base {
            command_catalog.insert(permission_base.clone(), PermissionCatalogSource::Command);
            registration
                .root
                .clone()
                .resolve_subcommand_permissions(&permission_base, &mut command_catalog)?
        } else {
            registration.root.clone()
        };
        let root_for_aliases = (!registration.aliases.is_empty()).then(|| root.clone());
        let mut graph = self.graph.clone();
        graph.register_root(root)?;
        for alias in registration.aliases {
            let root = root_for_aliases
                .as_ref()
                .ok_or(CommandRegistrationError::RootMustBeLiteral)?
                .clone()
                .with_literal_name(alias)
                .ok_or(CommandRegistrationError::RootMustBeLiteral)?;
            graph.register_root(root)?;
        }
        let validation = graph.validate();
        if !validation.is_empty() {
            return Err(CommandRegistrationError::InvalidGraphValidation(validation));
        }
        self.graph = graph;
        self.permission_catalog.extend(&command_catalog);
        Ok(())
    }

    /// Executes a command.
    pub fn handle_command(&self, sender: CommandSender, command: String, server: &Arc<Server>) {
        let mut context = CommandContext::new(sender.clone(), server.clone());

        let result = self.dispatch_with_context(command.clone(), &mut context);

        if let Err(error) = result {
            sender.send_failure_feedback(error.into_feedback(&command));
        }
    }

    /// Executes a command using an existing command context.
    pub fn dispatch_with_context(
        &self,
        command: String,
        context: &mut CommandContext,
    ) -> Result<CommandResult, CommandError> {
        let mut budget = CommandExecutionBudget::for_context(context);
        self.dispatch_with_budget(command, context, &mut budget)
    }

    pub(crate) fn dispatch_with_budget(
        &self,
        command: String,
        context: &mut CommandContext,
        budget: &mut CommandExecutionBudget,
    ) -> Result<CommandResult, CommandError> {
        let mut queue = VecDeque::new();
        let mut active = ActiveCommand::Borrowed { command, context };
        let mut total_success_count = 0_i32;

        loop {
            budget.consume()?;
            let step = {
                let (command, active_context) = active.parts();
                match self.graph.parse(command, active_context) {
                    Ok(parsed) => parsed.execute_step(active_context).map_err(|error| {
                        if parsed.invokes_result_callback_on_error() {
                            active_context.on_command_result(CommandCallbackResult {
                                success: false,
                                result: 0,
                            });
                        }
                        error
                    }),
                    Err(error) => {
                        active_context.on_command_result(CommandCallbackResult {
                            success: false,
                            result: 0,
                        });
                        Err(Self::parse_error_to_command_error(command, error))
                    }
                }
            };
            let step = match step {
                Ok(step) => step,
                Err(_error) if active.is_forked() => {
                    let Some(next) = queue.pop_front() else {
                        return Ok(CommandResult {
                            success_count: total_success_count,
                        });
                    };
                    active = ActiveCommand::Owned(next);
                    continue;
                }
                Err(error) => return Err(error),
            };

            match step {
                CommandExecutionStep::Complete(result) => {
                    active
                        .context_mut()
                        .on_command_result(CommandCallbackResult {
                            success: true,
                            result: result.success_count,
                        });
                    total_success_count = total_success_count
                        .saturating_add(execution_success_count(result, active.is_forked()));
                    let Some(next) = queue.pop_front() else {
                        return Ok(CommandResult {
                            success_count: total_success_count,
                        });
                    };
                    active = ActiveCommand::Owned(next);
                }
                CommandExecutionStep::Redirect {
                    command: next_command,
                    contexts,
                    forked,
                } => {
                    let next_forked = active.is_forked() || forked;
                    for context in contexts {
                        queue.push_back(QueuedCommand {
                            command: next_command.clone(),
                            context,
                            forked: next_forked,
                        });
                    }
                    let Some(next) = queue.pop_front() else {
                        return Ok(CommandResult {
                            success_count: total_success_count,
                        });
                    };
                    active = ActiveCommand::Owned(next);
                }
            }
        }
    }

    fn parse_error_to_command_error(input: &str, error: CommandParseError) -> CommandError {
        let cursor = error.cursor();
        CommandError::parse(Self::parse_error_message(error.kind()), input, cursor)
    }

    fn parse_error_message(kind: &CommandParseErrorKind) -> TextComponent {
        match kind {
            CommandParseErrorKind::EmptyCommand
            | CommandParseErrorKind::UnknownCommand
            | CommandParseErrorKind::IncompleteCommand => {
                TextComponent::from(&translations::COMMAND_UNKNOWN_COMMAND)
            }
            CommandParseErrorKind::ExpectedWhitespace | CommandParseErrorKind::TrailingData => {
                TextComponent::from(&translations::COMMAND_EXPECTED_SEPARATOR)
            }
            CommandParseErrorKind::ExpectedArgument => {
                TextComponent::from(&translations::COMMAND_UNKNOWN_ARGUMENT)
            }
            CommandParseErrorKind::ExpectedLiteral(literal) => {
                translations::ARGUMENT_LITERAL_INCORRECT
                    .message([TextComponent::from(literal.clone())])
                    .into()
            }
            CommandParseErrorKind::UnclosedQuote => {
                TextComponent::from(&translations::PARSING_QUOTE_EXPECTED_END)
            }
            CommandParseErrorKind::InvalidEscape(ch) => translations::PARSING_QUOTE_ESCAPE
                .message([TextComponent::from(ch.to_string())])
                .into(),
            CommandParseErrorKind::InvalidBool(value) => translations::PARSING_BOOL_INVALID
                .message([TextComponent::from(value.clone())])
                .into(),
            CommandParseErrorKind::InvalidAnchor(value) => translations::ARGUMENT_ANCHOR_INVALID
                .message([TextComponent::from(value.clone())])
                .into(),
            CommandParseErrorKind::InvalidInteger(value) => translations::PARSING_INT_INVALID
                .message([TextComponent::from(value.clone())])
                .into(),
            CommandParseErrorKind::InvalidLong(value) => {
                TextComponent::plain(format!("Invalid long integer '{value}'"))
            }
            CommandParseErrorKind::IntegerTooLow { value, min } => {
                translations::ARGUMENT_INTEGER_LOW
                    .message([
                        TextComponent::from(min.to_string()),
                        TextComponent::from(value.to_string()),
                    ])
                    .into()
            }
            CommandParseErrorKind::IntegerTooHigh { value, max } => {
                translations::ARGUMENT_INTEGER_BIG
                    .message([
                        TextComponent::from(max.to_string()),
                        TextComponent::from(value.to_string()),
                    ])
                    .into()
            }
            CommandParseErrorKind::LongTooLow { value, min } => {
                TextComponent::plain(format!("Long integer {value} must not be less than {min}"))
            }
            CommandParseErrorKind::LongTooHigh { value, max } => TextComponent::plain(format!(
                "Long integer {value} must not be greater than {max}"
            )),
            CommandParseErrorKind::InvalidFloat(value) => translations::PARSING_FLOAT_INVALID
                .message([TextComponent::from(value.clone())])
                .into(),
            CommandParseErrorKind::FloatTooLow { value, min } => translations::ARGUMENT_FLOAT_LOW
                .message([
                    TextComponent::from(min.to_string()),
                    TextComponent::from(value.to_string()),
                ])
                .into(),
            CommandParseErrorKind::FloatTooHigh { value, max } => translations::ARGUMENT_FLOAT_BIG
                .message([
                    TextComponent::from(max.to_string()),
                    TextComponent::from(value.to_string()),
                ])
                .into(),
            CommandParseErrorKind::InvalidGameMode(value) => {
                translations::ARGUMENT_GAMEMODE_INVALID
                    .message([TextComponent::from(value.clone())])
                    .into()
            }
            CommandParseErrorKind::InvalidPlayer(_) => {
                TextComponent::from(&translations::ARGUMENT_ENTITY_NOTFOUND_PLAYER)
            }
            CommandParseErrorKind::InvalidEntity(_) => {
                TextComponent::from(&translations::ARGUMENT_ENTITY_NOTFOUND_ENTITY)
            }
            CommandParseErrorKind::InvalidItem(value) => translations::ARGUMENT_ITEM_ID_INVALID
                .message([TextComponent::from(value.clone())])
                .into(),
            CommandParseErrorKind::InvalidWorld(value) => translations::ARGUMENT_DIMENSION_INVALID
                .message([TextComponent::from(value.clone())])
                .into(),
            CommandParseErrorKind::InvalidComponent(value) => {
                translations::ARGUMENT_COMPONENT_INVALID
                    .message([TextComponent::from(value.clone())])
                    .into()
            }
            CommandParseErrorKind::InvalidEntityType(value) => {
                TextComponent::plain(format!("Invalid entity type '{value}'"))
            }
            CommandParseErrorKind::InvalidEnchantment(value) => {
                TextComponent::plain(format!("Invalid enchantment '{value}'"))
            }
            CommandParseErrorKind::InvalidBiome(value) => {
                TextComponent::plain(format!("Invalid biome '{value}'"))
            }
            CommandParseErrorKind::InvalidBlockPredicate(value) => {
                TextComponent::plain(format!("Invalid block predicate: {value}"))
            }
            CommandParseErrorKind::InvalidNbtPath(value) => {
                TextComponent::plain(format!("Invalid NBT path: {value}"))
            }
            CommandParseErrorKind::InvalidStructure(value) => {
                TextComponent::plain(format!("Invalid structure '{value}'"))
            }
            CommandParseErrorKind::InvalidDomain(value) => {
                TextComponent::plain(format!("Invalid domain '{value}'"))
            }
            CommandParseErrorKind::InvalidVec3(value) => {
                TextComponent::plain(format!("Invalid position '{value}'"))
            }
            CommandParseErrorKind::InvalidBlockPos(value) => {
                TextComponent::plain(format!("Invalid block position '{value}'"))
            }
            CommandParseErrorKind::InvalidHeightmap(value) => {
                TextComponent::plain(format!("Invalid heightmap '{value}'"))
            }
            CommandParseErrorKind::InvalidRotation(value) => {
                TextComponent::plain(format!("Invalid rotation '{value}'"))
            }
            CommandParseErrorKind::InvalidSwizzle(value) => {
                TextComponent::plain(format!("Invalid swizzle '{value}'"))
            }
            CommandParseErrorKind::InvalidTime(value) => {
                TextComponent::plain(format!("Invalid time '{value}'"))
            }
            CommandParseErrorKind::InvalidPermissionKey(value) => {
                TextComponent::plain(format!("Invalid permission '{value}'"))
            }
            CommandParseErrorKind::InvalidPermissionExpression(value) => {
                TextComponent::plain(format!("Invalid permission expression: {value}"))
            }
            CommandParseErrorKind::InvalidPermissionMetadataExpression(value) => {
                TextComponent::plain(format!("Invalid permission metadata expression: {value}"))
            }
            CommandParseErrorKind::InvalidPermissionMetadataKey(value) => {
                TextComponent::plain(format!("Invalid permission metadata key '{value}'"))
            }
            CommandParseErrorKind::InvalidPermissionGroup(value) => {
                TextComponent::plain(format!("Invalid permission group '{value}'"))
            }
            CommandParseErrorKind::MissingCommandContext(name) => {
                TextComponent::plain(format!("Missing command context '{name}'"))
            }
            CommandParseErrorKind::ArgumentParserDidNotConsumeInput {
                argument,
                parsed_type,
            } => TextComponent::plain(format!(
                "Command argument parser '{parsed_type}' for '{argument}' did not consume input"
            )),
        }
    }

    /// Generates the `CCommands` packet visible to `context`.
    #[must_use]
    pub fn get_commands(&self, context: &dyn RequirementContext) -> CCommands {
        let mut nodes = Vec::with_capacity(1);
        nodes.push(CommandNode::new_root());

        let mut root_children = Vec::new();
        self.graph.usage(&mut nodes, &mut root_children, context);
        nodes[0].set_children(root_children);

        CCommands {
            root_index: 0,
            nodes,
        }
    }

    /// Handles a command suggestion request from a player.
    pub fn handle_player_suggestions(
        &self,
        player: &Arc<Player>,
        id: i32,
        command: &str,
        server: Arc<Server>,
    ) {
        let (suggestions, start, length) =
            self.handle_suggestions(CommandSender::Player(Arc::clone(player)), command, server);
        player.send_packet(CCommandSuggestions::new(id, start, length, suggestions));
    }

    /// Handles a command suggestion request from a player.
    pub fn handle_suggestions(
        &self,
        sender: CommandSender,
        command: &str,
        server: Arc<Server>,
    ) -> (Vec<SuggestionEntry>, i32, i32) {
        let mut catalog = self.permission_catalog.clone();
        server
            .permission_groups
            .register_catalog_entries(&mut catalog);
        let mut metadata_catalog = self.permission_metadata_catalog.clone();
        server
            .permission_groups
            .register_metadata_catalog_entries(&mut metadata_catalog);
        let mut context_catalog = self.permission_context_catalog.clone();
        server
            .permission_groups
            .register_context_catalog_entries(&mut context_catalog);
        let context = CommandContext::new(sender, server)
            .with_permission_catalog(catalog)
            .with_permission_metadata_catalog(metadata_catalog)
            .with_permission_context_catalog(context_catalog);

        self.graph
            .suggest(command, &context)
            .map_or((Vec::new(), 0, 0), |result| {
                (result.suggestions, result.start, result.length)
            })
    }
}

enum ActiveCommand<'a> {
    Borrowed {
        command: String,
        context: &'a mut CommandContext,
    },
    Owned(QueuedCommand),
}

struct QueuedCommand {
    command: String,
    context: CommandContext,
    forked: bool,
}

impl ActiveCommand<'_> {
    fn parts(&mut self) -> (&str, &mut CommandContext) {
        match self {
            Self::Borrowed { command, context } => (command, context),
            Self::Owned(QueuedCommand {
                command, context, ..
            }) => (command, context),
        }
    }

    fn context_mut(&mut self) -> &mut CommandContext {
        match self {
            Self::Borrowed { context, .. } => context,
            Self::Owned(QueuedCommand { context, .. }) => context,
        }
    }

    const fn is_forked(&self) -> bool {
        match self {
            Self::Borrowed { .. } => false,
            Self::Owned(QueuedCommand { forked, .. }) => *forked,
        }
    }
}

fn execution_success_count(result: CommandResult, forked: bool) -> i32 {
    if forked { 1 } else { result.success_count }
}

#[cfg(test)]
mod tests {
    use super::{
        CommandDispatcher, CommandExecutionBudget, CommandRegistration, CommandRegistrationError,
    };
    use crate::command::{
        error::CommandError,
        graph::{
            CommandGraphError, CommandParseErrorKind, CommandResult, StringParser, argument,
            literal,
        },
        reader::StringMode,
        requirement::{CommandInputContext, CommandSourceKind, PermissionExpr, RequirementContext},
    };
    use crate::permission::{
        PermissionCatalogSource, PermissionEntry, PermissionKey, PermissionSegment, PermissionSet,
    };
    use steel_registry::test_support::init_test_registry;
    use steel_utils::translations;
    use text_components::{TextComponent, content::Content};

    struct TestContext {
        permissions: PermissionSet,
    }

    impl RequirementContext for TestContext {
        fn source_kind(&self) -> CommandSourceKind {
            CommandSourceKind::Player
        }

        fn has_permission(&self, permission: &PermissionExpr) -> bool {
            self.permissions.allows(permission)
        }
    }

    impl CommandInputContext for TestContext {}

    fn player_context() -> TestContext {
        TestContext {
            permissions: PermissionSet::default(),
        }
    }

    fn player_context_with(permission: &str) -> TestContext {
        player_context_with_all([permission])
    }

    fn player_context_with_all<const N: usize>(permissions: [&str; N]) -> TestContext {
        player_context_with_entries(permissions.map(|permission| {
            PermissionEntry::allow(
                PermissionKey::parse(permission).expect("test permission key parses"),
            )
        }))
    }

    fn player_context_with_entries<const N: usize>(entries: [PermissionEntry; N]) -> TestContext {
        TestContext {
            permissions: PermissionSet::from_entries(entries),
        }
    }

    fn allow(permission: &str) -> PermissionEntry {
        PermissionEntry::allow(
            PermissionKey::parse(permission).expect("test permission key parses"),
        )
    }

    fn deny(permission: &str) -> PermissionEntry {
        PermissionEntry::deny(PermissionKey::parse(permission).expect("test permission key parses"))
    }

    fn text_content(component: &TextComponent) -> &str {
        let Content::Text { text } = &component.content else {
            panic!("component should be plain text");
        };
        text
    }

    fn translation_key(component: &TextComponent) -> &str {
        let Content::Translate(message) = &component.content else {
            panic!("component should be translated");
        };
        &message.key
    }

    #[test]
    fn command_execution_budget_rejects_after_limit() {
        let mut budget = CommandExecutionBudget::new(1);

        assert!(budget.consume().is_ok());
        assert!(budget.consume().is_err());
    }

    #[test]
    fn forked_execution_counts_completed_source_not_command_result() {
        assert_eq!(
            super::execution_success_count(CommandResult { success_count: 12 }, false),
            12
        );
        assert_eq!(
            super::execution_success_count(CommandResult { success_count: 12 }, true),
            1
        );
        assert_eq!(
            super::execution_success_count(CommandResult { success_count: 0 }, true),
            1
        );
    }

    #[test]
    fn dispatcher_applies_root_command_permissions() {
        init_test_registry();

        let dispatcher = CommandDispatcher::new().expect("built-in commands register");
        let player = player_context();

        assert!(dispatcher.graph.has_root("list", &player));
        assert!(!dispatcher.graph.has_root("give", &player));
        assert!(!dispatcher.graph.has_root("deop", &player));
        assert!(!dispatcher.graph.has_root("gamemode", &player));
        assert!(!dispatcher.graph.has_root("op", &player));
        assert!(!dispatcher.graph.has_root("tp", &player));
        assert!(!dispatcher.graph.has_root("teleport", &player));
        assert!(!dispatcher.graph.has_root("steelperms", &player));
        assert!(!dispatcher.graph.has_root("sp", &player));

        let give_player = player_context_with("minecraft.command.give");
        assert!(dispatcher.graph.has_root("give", &give_player));

        let op_player = player_context_with("minecraft.command.op");
        assert!(dispatcher.graph.has_root("op", &op_player));

        let deop_player = player_context_with("minecraft.command.deop");
        assert!(dispatcher.graph.has_root("deop", &deop_player));

        let gamemode_player = player_context_with("minecraft.command.gamemode");
        assert!(dispatcher.graph.has_root("gamemode", &gamemode_player));
        assert!(!dispatcher.graph.has_root("tp", &gamemode_player));

        let gamemode_creative_player = player_context_with("minecraft.command.gamemode.creative");
        assert!(
            dispatcher
                .graph
                .has_root("gamemode", &gamemode_creative_player)
        );

        let teleport_player = player_context_with("minecraft.command.teleport");
        assert!(dispatcher.graph.has_root("tp", &teleport_player));
        assert!(dispatcher.graph.has_root("teleport", &teleport_player));

        let experience_player = player_context_with("minecraft.command.experience");
        assert!(dispatcher.graph.has_root("xp", &experience_player));
        assert!(dispatcher.graph.has_root("experience", &experience_player));

        let steelperms_player = player_context_with("steel.command.steelperms");
        assert!(dispatcher.graph.has_root("steelperms", &steelperms_player));
        assert!(dispatcher.graph.has_root("sp", &steelperms_player));
    }

    #[test]
    fn dispatcher_applies_derived_subcommand_permissions() {
        init_test_registry();

        let dispatcher = CommandDispatcher::new().expect("built-in commands register");
        let tick_root_player = player_context_with("minecraft.command.tick");
        assert!(dispatcher.graph.has_root("tick", &tick_root_player));
        assert!(
            dispatcher
                .graph
                .parse("tick freeze", &tick_root_player)
                .is_ok()
        );

        let tick_freeze_player = player_context_with("minecraft.command.tick.freeze");
        assert!(dispatcher.graph.has_root("tick", &tick_freeze_player));
        assert!(
            dispatcher
                .graph
                .parse("tick freeze", &tick_freeze_player)
                .is_ok()
        );

        let tick_root_without_freeze = player_context_with_entries([
            allow("minecraft.command.tick"),
            deny("minecraft.command.tick.freeze"),
        ]);
        assert!(
            dispatcher
                .graph
                .parse("tick freeze", &tick_root_without_freeze)
                .is_err()
        );
    }

    #[test]
    fn derived_subcommand_permissions_use_root_permission_override() {
        let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
        let permission =
            super::command_permission_key(&minecraft, "long").expect("permission key parses");
        let mut dispatcher = CommandDispatcher::new_empty();

        let registration = CommandRegistration::new(
            literal("short").then(
                literal("child")
                    .requires_subcommand_permission()
                    .executes(|_, _| Ok(CommandResult::success())),
            ),
            minecraft,
        )
        .permission(permission)
        .alias("alias")
        .expect("alias parses");

        dispatcher
            .register_command(registration)
            .expect("command registers");

        let root_player = player_context_with("minecraft.command.long");
        assert!(dispatcher.graph.parse("short child", &root_player).is_ok());

        let child_player = player_context_with("minecraft.command.long.child");
        assert!(dispatcher.graph.parse("short child", &child_player).is_ok());
        assert!(dispatcher.graph.parse("alias child", &child_player).is_ok());
    }

    #[test]
    fn additional_subcommand_permissions_require_root_and_child() {
        let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
        let mut dispatcher = CommandDispatcher::new_empty();

        let registration = CommandRegistration::new(
            literal("root")
                .then(
                    literal("view")
                        .requires_subcommand_permission()
                        .executes(|_, _| Ok(CommandResult::success())),
                )
                .then(
                    literal("admin")
                        .requires_additional_subcommand_permission()
                        .executes(|_, _| Ok(CommandResult::success())),
                ),
            minecraft,
        );

        dispatcher
            .register_command(registration)
            .expect("command registers");

        let root_player = player_context_with("minecraft.command.root");
        assert!(dispatcher.graph.has_root("root", &root_player));
        assert!(dispatcher.graph.parse("root admin", &root_player).is_err());

        let child_player = player_context_with("minecraft.command.root.admin");
        assert!(!dispatcher.graph.has_root("root", &child_player));

        let view_and_admin_player = player_context_with_all([
            "minecraft.command.root.view",
            "minecraft.command.root.admin",
        ]);
        assert!(dispatcher.graph.has_root("root", &view_and_admin_player));
        assert!(
            dispatcher
                .graph
                .parse("root view", &view_and_admin_player)
                .is_ok()
        );
        assert!(
            dispatcher
                .graph
                .parse("root admin", &view_and_admin_player)
                .is_err()
        );

        let admin_player =
            player_context_with_all(["minecraft.command.root", "minecraft.command.root.admin"]);
        assert!(dispatcher.graph.parse("root admin", &admin_player).is_ok());
    }

    #[test]
    fn dispatcher_catalog_tracks_command_permissions() {
        init_test_registry();

        let dispatcher = CommandDispatcher::new().expect("built-in commands register");
        let suggestions = dispatcher
            .permission_catalog
            .suggestions("minecraft.command.gamemode");

        assert_eq!(
            suggestions,
            vec![
                "minecraft.command.gamemode".to_owned(),
                "minecraft.command.gamemode.adventure".to_owned(),
                "minecraft.command.gamemode.creative".to_owned(),
                "minecraft.command.gamemode.spectator".to_owned(),
                "minecraft.command.gamemode.survival".to_owned(),
            ]
        );
        assert!(
            dispatcher
                .permission_catalog
                .entries()
                .all(|entry| entry.sources().contains(&PermissionCatalogSource::Command))
        );
        assert!(
            !dispatcher
                .permission_catalog
                .suggestions("steel.command.sp")
                .iter()
                .any(|key| key == "steel.command.sp")
        );
    }

    #[test]
    fn built_in_command_graph_has_no_validation_diagnostics() {
        init_test_registry();

        let dispatcher = CommandDispatcher::new().expect("built-in commands register");
        let validation = dispatcher.graph.validate();

        assert!(validation.is_empty());
    }

    #[test]
    fn command_registration_rejects_ambiguous_graph_atomically() {
        let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
        let mut dispatcher = CommandDispatcher::new_empty();
        dispatcher
            .register_command(
                CommandRegistration::new(
                    literal("other").executes(|_, _| Ok(CommandResult::success())),
                    minecraft.clone(),
                )
                .public(),
            )
            .expect("existing command registers");

        let registration = CommandRegistration::new(
            literal("root").then_all([
                argument("value", StringParser::new(StringMode::SingleWord))
                    .executes(|_, _| Ok(CommandResult::success())),
                literal("run").executes(|_, _| Ok(CommandResult::success())),
            ]),
            minecraft,
        )
        .public();

        let Err(error) = dispatcher.register_command(registration) else {
            panic!("ambiguous command should fail registration");
        };
        let CommandRegistrationError::InvalidGraphValidation(validation) = error else {
            panic!("expected graph validation error, got {error:?}");
        };
        assert_eq!(validation.ambiguities().len(), 1);

        let player = player_context();
        assert!(!dispatcher.graph.has_root("root", &player));
        assert!(dispatcher.graph.has_root("other", &player));
    }

    #[test]
    fn aliases_validate_as_command_literals_not_permission_segments() {
        let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
        let mut dispatcher = CommandDispatcher::new_empty();
        let registration = CommandRegistration::new(
            literal("root").executes(|_, _| Ok(CommandResult::success())),
            minecraft,
        )
        .alias("Alias")
        .expect("alias is a valid command literal");

        dispatcher
            .register_command(registration)
            .expect("command registers");

        let player = player_context_with("minecraft.command.root");
        assert!(dispatcher.graph.has_root("Alias", &player));
    }

    #[test]
    fn public_commands_do_not_silently_discard_derived_permission_markers() {
        let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
        let mut dispatcher = CommandDispatcher::new_empty();
        let registration = CommandRegistration::new(
            literal("root").then(literal("child").requires_subcommand_permission()),
            minecraft,
        )
        .public();

        let Err(error) = dispatcher.register_command(registration) else {
            panic!("public derived permission marker should fail registration");
        };

        assert!(matches!(
            error,
            CommandRegistrationError::InvalidGraph(
                CommandGraphError::UnresolvedDerivedPermission { .. }
            )
        ));
    }

    #[test]
    fn public_commands_do_not_require_permission_segment_literals() {
        let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
        let mut dispatcher = CommandDispatcher::new_empty();
        let registration = CommandRegistration::new(
            literal("Visible").executes(|_, _| Ok(CommandResult::success())),
            minecraft,
        )
        .public();

        dispatcher
            .register_command(registration)
            .expect("public command registers without permission key derivation");

        let player = player_context();
        assert!(dispatcher.graph.has_root("Visible", &player));
        assert!(dispatcher.graph.parse("Visible", &player).is_ok());
    }

    #[test]
    fn aliases_reject_invalid_command_literals() {
        let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
        let Err(error) = CommandRegistration::new(literal("root"), minecraft).alias("bad alias")
        else {
            panic!("alias with whitespace should fail");
        };

        assert!(matches!(
            error,
            CommandRegistrationError::InvalidGraph(CommandGraphError::InvalidLiteralName { .. })
        ));
    }

    #[test]
    fn failed_alias_registration_does_not_leave_primary_root() {
        let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
        let mut dispatcher = CommandDispatcher::new_empty();
        dispatcher
            .register_command(
                CommandRegistration::new(
                    literal("other").executes(|_, _| Ok(CommandResult::success())),
                    minecraft.clone(),
                )
                .public(),
            )
            .expect("existing command registers");

        let registration = CommandRegistration::new(
            literal("root").executes(|_, _| Ok(CommandResult::success())),
            minecraft,
        )
        .public()
        .alias("other")
        .expect("alias literal parses");
        let Err(error) = dispatcher.register_command(registration) else {
            panic!("colliding alias should reject registration");
        };

        assert!(matches!(
            error,
            CommandRegistrationError::InvalidGraph(CommandGraphError::LiteralCollision { .. })
        ));
        let player = player_context();
        assert!(!dispatcher.graph.has_root("root", &player));
        assert!(dispatcher.graph.has_root("other", &player));
    }

    #[test]
    fn parse_error_mapping_uses_vanilla_integer_bound_order() {
        let error = super::CommandParseError::new(
            CommandParseErrorKind::IntegerTooLow { value: 1, min: 5 },
            5,
        );

        let CommandError::Parse(report) =
            CommandDispatcher::parse_error_to_command_error("test 1", error)
        else {
            panic!("parse error should become structured feedback");
        };
        let feedback = report.into_feedback();
        let Content::Translate(message) = &feedback.primary.content else {
            panic!("primary message should be translated");
        };
        let args = message.args.as_ref().expect("integer low has arguments");

        assert_eq!(message.key.as_ref(), translations::ARGUMENT_INTEGER_LOW.0);
        assert_eq!(text_content(&args[0]), "5");
        assert_eq!(text_content(&args[1]), "1");
    }

    #[test]
    fn parse_error_mapping_uses_vanilla_parser_translations() {
        assert_eq!(
            translation_key(&CommandDispatcher::parse_error_message(
                &CommandParseErrorKind::UnknownCommand
            )),
            translations::COMMAND_UNKNOWN_COMMAND.0
        );
        assert_eq!(
            translation_key(&CommandDispatcher::parse_error_message(
                &CommandParseErrorKind::TrailingData
            )),
            translations::COMMAND_EXPECTED_SEPARATOR.0
        );
        assert_eq!(
            translation_key(&CommandDispatcher::parse_error_message(
                &CommandParseErrorKind::InvalidBool("maybe".to_owned())
            )),
            translations::PARSING_BOOL_INVALID.0
        );
    }
}
