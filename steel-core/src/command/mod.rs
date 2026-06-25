//! This module contains everything needed for commands (e.g., parsing, execution, and sender handling).
pub mod commands;
pub mod context;
pub mod error;
pub mod graph;
pub mod parsers;
pub mod reader;
pub mod requirement;
pub mod sender;

use steel_protocol::packets::game::{CCommandSuggestions, CCommands, CommandNode, SuggestionEntry};
use text_components::{Modifier, TextComponent, format::Color};

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandGraph, CommandNodeBuilder, CommandParseError, CommandParseErrorKind, CommandResult,
};
use crate::command::requirement::RequirementContext;
use crate::command::sender::CommandSender;
use crate::permission::{PermissionKey, PermissionKeyError, PermissionSegment};
use crate::player::Player;
use crate::server::Server;
use std::{error::Error, fmt, sync::Arc};

/// Parses and dispatches commands through the command graph.
#[derive(Default)]
pub struct CommandDispatcher {
    /// Dynamic command graph.
    graph: CommandGraph,
}

struct CommandRegistration {
    root: CommandNodeBuilder,
    namespace: PermissionSegment,
    permission: CommandPermissionMode,
    aliases: Vec<PermissionSegment>,
}

impl CommandRegistration {
    const fn new(root: CommandNodeBuilder, namespace: PermissionSegment) -> Self {
        Self {
            root,
            namespace,
            permission: CommandPermissionMode::Auto,
            aliases: Vec::new(),
        }
    }

    fn public(mut self) -> Self {
        self.permission = CommandPermissionMode::Public;
        self
    }

    fn permission(mut self, permission: PermissionKey) -> Self {
        self.permission = CommandPermissionMode::Override(permission);
        self
    }

    fn alias(mut self, alias: &str) -> Result<Self, CommandRegistrationError> {
        self.aliases.push(PermissionSegment::parse(alias)?);
        Ok(self)
    }

    fn resolved_permission(&self) -> Result<Option<PermissionKey>, CommandRegistrationError> {
        match &self.permission {
            CommandPermissionMode::Auto => {
                let command_name = self
                    .root
                    .literal_name()
                    .ok_or(CommandRegistrationError::RootMustBeLiteral)?;
                Ok(Some(command_permission_key(&self.namespace, command_name)?))
            }
            CommandPermissionMode::Public => Ok(None),
            CommandPermissionMode::Override(permission) => Ok(Some(permission.clone())),
        }
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
}

impl fmt::Display for CommandRegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RootMustBeLiteral => write!(f, "command root must be a literal node"),
            Self::InvalidPermissionKey(error) => write!(f, "{error}"),
        }
    }
}

impl Error for CommandRegistrationError {}

impl From<PermissionKeyError> for CommandRegistrationError {
    fn from(value: PermissionKeyError) -> Self {
        Self::InvalidPermissionKey(value)
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

impl CommandDispatcher {
    /// Creates a new command dispatcher with built-in commands.
    ///
    /// # Errors
    ///
    /// Returns an error when a built-in command registration is invalid.
    pub fn new() -> Result<Self, CommandRegistrationError> {
        let mut dispatcher = CommandDispatcher::new_empty();
        let minecraft = PermissionSegment::parse("minecraft")?;
        let steel = PermissionSegment::parse("steel")?;

        dispatcher.register_command(CommandRegistration::new(
            commands::clear::command(),
            minecraft.clone(),
        ))?;
        dispatcher.register_command(CommandRegistration::new(
            commands::domain::command(),
            steel.clone(),
        ))?;
        dispatcher.register_command(CommandRegistration::new(
            commands::enchant::command(),
            minecraft.clone(),
        ))?;
        dispatcher.register_command(CommandRegistration::new(
            commands::execute::command(),
            minecraft.clone(),
        ))?;
        dispatcher.register_command(CommandRegistration::new(
            commands::fly::command(),
            steel.clone(),
        ))?;
        dispatcher.register_command(CommandRegistration::new(
            commands::gamemode::command(),
            minecraft.clone(),
        ))?;
        dispatcher.register_command(CommandRegistration::new(
            commands::gamerule::command(),
            minecraft.clone(),
        ))?;
        dispatcher.register_command(CommandRegistration::new(
            commands::kill::command(),
            minecraft.clone(),
        ))?;
        dispatcher.register_command(
            CommandRegistration::new(commands::list::command(), minecraft.clone()).public(),
        )?;
        dispatcher.register_command(CommandRegistration::new(
            commands::locate::command(),
            minecraft.clone(),
        ))?;
        dispatcher.register_command(CommandRegistration::new(
            commands::give::command(),
            minecraft.clone(),
        ))?;
        dispatcher.register_command(CommandRegistration::new(
            commands::seed::command(),
            minecraft.clone(),
        ))?;
        dispatcher.register_command(CommandRegistration::new(
            commands::setworldspawn::command(),
            minecraft.clone(),
        ))?;
        dispatcher.register_command(CommandRegistration::new(
            commands::stop::command(),
            minecraft.clone(),
        ))?;
        dispatcher.register_command(CommandRegistration::new(
            commands::summon::command(),
            minecraft.clone(),
        ))?;
        dispatcher.register_command(CommandRegistration::new(
            commands::tellraw::command(),
            minecraft.clone(),
        ))?;
        dispatcher.register_command(CommandRegistration::new(
            commands::tick::command(),
            minecraft.clone(),
        ))?;
        dispatcher.register_command(CommandRegistration::new(
            commands::time::command(),
            minecraft.clone(),
        ))?;
        dispatcher.register_command(
            CommandRegistration::new(commands::tp::command(), minecraft.clone())
                .permission(command_permission_key(&minecraft, "teleport")?)
                .alias("teleport")?,
        )?;
        dispatcher.register_command(CommandRegistration::new(
            commands::weather::command(),
            minecraft.clone(),
        ))?;
        dispatcher.register_command(CommandRegistration::new(
            commands::difficulty::command(),
            minecraft.clone(),
        ))?;
        dispatcher.register_command(CommandRegistration::new(commands::steel::command(), steel))?;
        dispatcher.register_command(
            CommandRegistration::new(commands::xp::command(), minecraft.clone())
                .permission(command_permission_key(&minecraft, "experience")?)
                .alias("experience")?,
        )?;
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
        let permission = registration.resolved_permission()?;
        self.register_root(registration.root.clone(), permission.clone());
        for alias in registration.aliases {
            let root = registration
                .root
                .clone()
                .with_literal_name(alias.as_str())
                .ok_or(CommandRegistrationError::RootMustBeLiteral)?;
            self.register_root(root, permission.clone());
        }
        Ok(())
    }

    fn register_root(&mut self, root: CommandNodeBuilder, permission: Option<PermissionKey>) {
        let root = match permission {
            Some(permission) => root.requires_permission(permission),
            None => root,
        };
        self.graph.register_root(root);
    }

    /// Executes a command.
    pub fn handle_command(&self, sender: CommandSender, command: String, server: &Arc<Server>) {
        let mut context = CommandContext::new(sender.clone(), server.clone());

        let result = self.dispatch_with_context(&command, &mut context);

        if let Err(error) = result {
            let text = match error {
                CommandError::InvalidConsumption(s) => {
                    log::error!(
                        "Error while parsing command \"{command}\": {s:?} was consumed, but couldn't be parsed"
                    );
                    TextComponent::const_plain("Internal error (See logs for details)")
                }
                CommandError::InvalidRequirement => {
                    log::error!(
                        "Error while parsing command \"{command}\": a requirement that was expected was not met."
                    );
                    TextComponent::const_plain("Internal error (See logs for details)")
                }
                CommandError::PermissionDenied => {
                    log::warn!("Permission denied for command \"{command}\"");
                    TextComponent::const_plain(
                        "I'm sorry, but you do not have permission to perform this command. Please contact the server administrator if you believe this is an error.",
                    )
                }
                CommandError::CommandFailed(text_component) => *text_component,
            };

            // TODO: Use vanilla error messages
            sender.send_message(&text.color(Color::Red));
        }
    }

    /// Executes a command using an existing command context.
    pub fn dispatch_with_context(
        &self,
        command: &str,
        context: &mut CommandContext,
    ) -> Result<CommandResult, CommandError> {
        self.execute_graph(command, context)
    }

    fn execute_graph(
        &self,
        command: &str,
        context: &mut CommandContext,
    ) -> Result<CommandResult, CommandError> {
        self.graph
            .parse(command, context)
            .map_err(Self::parse_error_to_command_error)?
            .execute_with_dispatcher(context, |command, context| {
                self.dispatch_with_context(command, context)
            })
    }

    fn parse_error_to_command_error(error: CommandParseError) -> CommandError {
        let cursor = error.cursor();
        let message = match error.kind() {
            CommandParseErrorKind::EmptyCommand => "Empty command".to_owned(),
            CommandParseErrorKind::ExpectedWhitespace => "Expected whitespace".to_owned(),
            CommandParseErrorKind::ExpectedArgument => "Expected argument".to_owned(),
            CommandParseErrorKind::ExpectedLiteral(literal) => {
                format!("Expected literal '{literal}'")
            }
            CommandParseErrorKind::UnknownCommand => "Unknown command".to_owned(),
            CommandParseErrorKind::IncompleteCommand => "Incomplete command".to_owned(),
            CommandParseErrorKind::TrailingData => "Trailing data found".to_owned(),
            CommandParseErrorKind::UnclosedQuote => "Unclosed quoted string".to_owned(),
            CommandParseErrorKind::InvalidEscape(ch) => {
                format!("Invalid escape sequence '\\{ch}'")
            }
            CommandParseErrorKind::InvalidBool(value) => {
                format!("Invalid boolean '{value}'")
            }
            CommandParseErrorKind::InvalidAnchor(value) => {
                format!("Invalid entity anchor '{value}'")
            }
            CommandParseErrorKind::InvalidInteger(value) => {
                format!("Invalid integer '{value}'")
            }
            CommandParseErrorKind::IntegerTooLow { value, min } => {
                format!("Integer {value} must be at least {min}")
            }
            CommandParseErrorKind::IntegerTooHigh { value, max } => {
                format!("Integer {value} must be at most {max}")
            }
            CommandParseErrorKind::InvalidFloat(value) => {
                format!("Invalid float '{value}'")
            }
            CommandParseErrorKind::FloatTooLow { value, min } => {
                format!("Float {value} must be at least {min}")
            }
            CommandParseErrorKind::FloatTooHigh { value, max } => {
                format!("Float {value} must be at most {max}")
            }
            CommandParseErrorKind::InvalidGameMode(value) => {
                format!("Invalid game mode '{value}'")
            }
            CommandParseErrorKind::InvalidPlayer(value) => {
                format!("Invalid player '{value}'")
            }
            CommandParseErrorKind::InvalidEntity(value) => {
                format!("Invalid entity '{value}'")
            }
            CommandParseErrorKind::InvalidEntityType(value) => {
                format!("Invalid entity type '{value}'")
            }
            CommandParseErrorKind::InvalidItem(value) => {
                format!("Invalid item '{value}'")
            }
            CommandParseErrorKind::InvalidEnchantment(value) => {
                format!("Invalid enchantment '{value}'")
            }
            CommandParseErrorKind::InvalidStructure(value) => {
                format!("Invalid structure '{value}'")
            }
            CommandParseErrorKind::InvalidDomain(value) => {
                format!("Invalid domain '{value}'")
            }
            CommandParseErrorKind::InvalidWorld(value) => {
                format!("Invalid world '{value}'")
            }
            CommandParseErrorKind::InvalidVec3(value) => {
                format!("Invalid position '{value}'")
            }
            CommandParseErrorKind::InvalidBlockPos(value) => {
                format!("Invalid block position '{value}'")
            }
            CommandParseErrorKind::InvalidRotation(value) => {
                format!("Invalid rotation '{value}'")
            }
            CommandParseErrorKind::InvalidComponent(value) => {
                format!("Invalid component '{value}'")
            }
            CommandParseErrorKind::InvalidTime(value) => {
                format!("Invalid time '{value}'")
            }
            CommandParseErrorKind::MissingCommandContext(name) => {
                format!("Missing command context '{name}'")
            }
        };

        CommandError::CommandFailed(Box::new(TextComponent::plain(format!(
            "{message} at position {cursor}"
        ))))
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
    use crate::command::{
        CommandDispatcher,
        requirement::{CommandSourceKind, PermissionExpr, RequirementContext},
    };
    use crate::permission::{PermissionEntry, PermissionKey, PermissionSet};
    use steel_registry::test_support::init_test_registry;

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

    fn player_context() -> TestContext {
        TestContext {
            permissions: PermissionSet::default(),
        }
    }

    fn player_context_with(permission: &str) -> TestContext {
        TestContext {
            permissions: PermissionSet::from_entries([PermissionEntry::allow(
                PermissionKey::parse(permission).expect("test permission key parses"),
            )]),
        }
    }

    #[test]
    fn dispatcher_applies_root_command_permissions() {
        init_test_registry();

        let dispatcher = CommandDispatcher::new().expect("built-in commands register");
        let player = player_context();

        assert!(dispatcher.graph.has_root("list", &player));
        assert!(!dispatcher.graph.has_root("give", &player));
        assert!(!dispatcher.graph.has_root("gamemode", &player));
        assert!(!dispatcher.graph.has_root("tp", &player));
        assert!(!dispatcher.graph.has_root("teleport", &player));

        let give_player = player_context_with("minecraft.command.give");
        assert!(dispatcher.graph.has_root("give", &give_player));

        let gamemode_player = player_context_with("minecraft.command.gamemode");
        assert!(dispatcher.graph.has_root("gamemode", &gamemode_player));
        assert!(!dispatcher.graph.has_root("tp", &gamemode_player));

        let teleport_player = player_context_with("minecraft.command.teleport");
        assert!(dispatcher.graph.has_root("tp", &teleport_player));
        assert!(dispatcher.graph.has_root("teleport", &teleport_player));

        let experience_player = player_context_with("minecraft.command.experience");
        assert!(dispatcher.graph.has_root("xp", &experience_player));
        assert!(dispatcher.graph.has_root("experience", &experience_player));
    }
}
