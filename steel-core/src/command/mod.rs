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

use steel_protocol::packets::game::{CCommandSuggestions, CCommands, CommandNode, SuggestionEntry};
use steel_utils::translations;
use text_components::TextComponent;

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandFuture, CommandGraph, CommandGraphError, CommandNodeBuilder, CommandParseError,
    CommandParseErrorKind, CommandResult, validate_command_node_name,
};
use crate::command::requirement::RequirementContext;
use crate::command::sender::CommandSender;
use crate::permission::{PermissionKey, PermissionKeyError, PermissionSegment};
use crate::player::Player;
use crate::server::Server;
use std::{error::Error, fmt, sync::Arc};

pub(crate) use executor::CommandQueue;

/// Parses and dispatches commands through the command graph.
#[derive(Clone, Default)]
pub struct CommandDispatcher {
    /// Dynamic command graph.
    graph: CommandGraph,
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

    fn resolved_permission_base(&self) -> Result<PermissionKey, CommandRegistrationError> {
        match &self.permission {
            CommandPermissionMode::Auto | CommandPermissionMode::Public => {
                let command_name = self
                    .root
                    .literal_name()
                    .ok_or(CommandRegistrationError::RootMustBeLiteral)?;
                Ok(command_permission_key(&self.namespace, command_name)?)
            }
            CommandPermissionMode::Override(permission) => Ok(permission.clone()),
        }
    }

    const fn has_root_permission(&self) -> bool {
        !matches!(self.permission, CommandPermissionMode::Public)
    }
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
}

impl fmt::Display for CommandRegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RootMustBeLiteral => write!(f, "command root must be a literal node"),
            Self::InvalidPermissionKey(error) => write!(f, "{error}"),
            Self::InvalidGraph(error) => write!(f, "{error}"),
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
        }
    }

    fn register_command(
        &mut self,
        registration: CommandRegistration,
    ) -> Result<(), CommandRegistrationError> {
        let permission_base = registration.resolved_permission_base()?;
        let permission = registration
            .has_root_permission()
            .then(|| permission_base.clone());
        let root = registration
            .root
            .clone()
            .resolve_subcommand_permissions(&permission_base)?;
        self.register_root(root, permission.clone())?;
        for alias in registration.aliases {
            let root = registration
                .root
                .clone()
                .with_literal_name(alias)
                .ok_or(CommandRegistrationError::RootMustBeLiteral)?
                .resolve_subcommand_permissions(&permission_base)?;
            self.register_root(root, permission.clone())?;
        }
        Ok(())
    }

    fn register_root(
        &mut self,
        root: CommandNodeBuilder,
        permission: Option<PermissionKey>,
    ) -> Result<(), CommandRegistrationError> {
        let root = match permission {
            Some(permission) => root.requires_permission(permission),
            None => root,
        };
        self.graph.register_root(root)?;
        Ok(())
    }

    /// Executes a command.
    pub async fn handle_command(
        &self,
        sender: CommandSender,
        command: String,
        server: &Arc<Server>,
    ) {
        let mut context = CommandContext::new(sender.clone(), server.clone());

        let result = self
            .dispatch_with_context(command.clone(), &mut context)
            .await;

        if let Err(error) = result {
            sender.send_failure_feedback(error.into_feedback(&command));
        }
    }

    /// Executes a command using an existing command context.
    pub fn dispatch_with_context<'a>(
        &'a self,
        command: String,
        context: &'a mut CommandContext,
    ) -> CommandFuture<'a> {
        Box::pin(async move { self.execute_graph(&command, context).await })
    }

    async fn execute_graph(
        &self,
        command: &str,
        context: &mut CommandContext,
    ) -> Result<CommandResult, CommandError> {
        self.graph
            .parse(command, context)
            .map_err(|error| Self::parse_error_to_command_error(command, error))?
            .execute_with_dispatcher(context, self)
            .await
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
            CommandParseErrorKind::InvalidRotation(value) => {
                TextComponent::plain(format!("Invalid rotation '{value}'"))
            }
            CommandParseErrorKind::InvalidTime(value) => {
                TextComponent::plain(format!("Invalid time '{value}'"))
            }
            CommandParseErrorKind::InvalidPermissionKey(value) => {
                TextComponent::plain(format!("Invalid permission '{value}'"))
            }
            CommandParseErrorKind::InvalidPermissionGroup(value) => {
                TextComponent::plain(format!("Invalid permission group '{value}'"))
            }
            CommandParseErrorKind::MissingCommandContext(name) => {
                TextComponent::plain(format!("Missing command context '{name}'"))
            }
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
        let context = CommandContext::new(sender, server);

        self.graph
            .suggest(command, &context)
            .map_or((Vec::new(), 0, 0), |result| {
                (result.suggestions, result.start, result.length)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{CommandDispatcher, CommandRegistration, CommandRegistrationError};
    use crate::command::{
        error::CommandError,
        graph::{CommandGraphError, CommandParseErrorKind, CommandResult, literal},
        requirement::{CommandInputContext, CommandSourceKind, PermissionExpr, RequirementContext},
    };
    use crate::permission::{PermissionEntry, PermissionKey, PermissionSegment, PermissionSet};
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
        TestContext {
            permissions: PermissionSet::from_entries(permissions.map(|permission| {
                PermissionEntry::allow(
                    PermissionKey::parse(permission).expect("test permission key parses"),
                )
            })),
        }
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
    fn dispatcher_applies_root_command_permissions() {
        init_test_registry();

        let dispatcher = CommandDispatcher::new().expect("built-in commands register");
        let player = player_context();

        assert!(dispatcher.graph.has_root("list", &player));
        assert!(!dispatcher.graph.has_root("give", &player));
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

        let gamemode_player = player_context_with("minecraft.command.gamemode");
        assert!(dispatcher.graph.has_root("gamemode", &gamemode_player));
        assert!(!dispatcher.graph.has_root("tp", &gamemode_player));

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

        dispatcher
            .graph
            .parse("tick freeze", &tick_root_player)
            .expect_err("subcommand permission should be required");

        let tick_freeze_player =
            player_context_with_all(["minecraft.command.tick", "minecraft.command.tick.freeze"]);
        assert!(
            dispatcher
                .graph
                .parse("tick freeze", &tick_freeze_player)
                .is_ok()
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
        let error = dispatcher
            .graph
            .parse("short child", &root_player)
            .expect_err("subcommand should use overridden root permission");
        assert_eq!(error.kind(), &CommandParseErrorKind::UnknownCommand);

        let child_player =
            player_context_with_all(["minecraft.command.long", "minecraft.command.long.child"]);
        assert!(dispatcher.graph.parse("short child", &child_player).is_ok());
        assert!(dispatcher.graph.parse("alias child", &child_player).is_ok());
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
