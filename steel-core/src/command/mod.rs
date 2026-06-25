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
    CommandGraph, CommandParseError, CommandParseErrorKind, CommandResult,
};
use crate::command::requirement::RequirementContext;
use crate::command::sender::CommandSender;
use crate::player::Player;
use crate::server::Server;
use std::sync::Arc;

/// Parses and dispatches commands through the command graph.
#[derive(Default)]
pub struct CommandDispatcher {
    /// Dynamic command graph.
    graph: CommandGraph,
}

impl CommandDispatcher {
    /// Creates a new command dispatcher with vanilla commands.
    #[must_use]
    pub fn new() -> Self {
        let mut dispatcher = CommandDispatcher::new_empty();
        dispatcher.graph.register_root(commands::clear::command());
        dispatcher.graph.register_root(commands::domain::command());
        dispatcher.graph.register_root(commands::enchant::command());
        dispatcher.graph.register_root(commands::execute::command());
        dispatcher.graph.register_root(commands::fly::command());
        dispatcher
            .graph
            .register_root(commands::gamemode::command());
        dispatcher
            .graph
            .register_root(commands::gamerule::command());
        dispatcher.graph.register_root(commands::kill::command());
        dispatcher.graph.register_root(commands::list::command());
        dispatcher.graph.register_root(commands::locate::command());
        dispatcher.graph.register_root(commands::give::command());
        dispatcher.graph.register_root(commands::seed::command());
        dispatcher
            .graph
            .register_root(commands::setworldspawn::command());
        dispatcher.graph.register_root(commands::stop::command());
        dispatcher.graph.register_root(commands::summon::command());
        dispatcher.graph.register_root(commands::tellraw::command());
        dispatcher.graph.register_root(commands::tick::command());
        dispatcher.graph.register_root(commands::time::command());
        dispatcher.graph.register_root(commands::tp::command());
        dispatcher
            .graph
            .register_root(commands::tp::teleport_command());
        dispatcher.graph.register_root(commands::weather::command());
        dispatcher
            .graph
            .register_root(commands::difficulty::command());
        dispatcher.graph.register_root(commands::steel::command());
        dispatcher.graph.register_root(commands::xp::command());
        dispatcher
            .graph
            .register_root(commands::xp::experience_command());
        dispatcher
    }

    /// Creates a new command dispatcher with no commands.
    #[must_use]
    pub const fn new_empty() -> Self {
        CommandDispatcher {
            graph: CommandGraph::new(),
        }
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
