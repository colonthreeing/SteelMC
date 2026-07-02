use std::{borrow::Cow, sync::Arc};

use steel_protocol::packets::game::{CCommandSuggestions, CCommands, CommandNode, SuggestionEntry};
use steel_utils::translations;
use text_components::TextComponent;
use text_components::translation::TranslatedMessage;

use crate::command::commands;
use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{CommandGraph, CommandParseError, CommandParseErrorKind};
use crate::command::registration::{
    CommandRegistration, CommandRegistrationError, entity_selector_advanced_permission_key,
    entity_selector_permission_key,
};
use crate::command::requirement::RequirementContext;
use crate::command::sender::CommandSender;
use crate::permission::{
    PermissionCatalog, PermissionCatalogSource, PermissionContextCatalog, PermissionMetadataCatalog,
};
use crate::player::Player;
use crate::server::Server;

/// Parses and dispatches commands through the command graph.
#[derive(Clone, Default)]
pub struct CommandDispatcher {
    /// Dynamic command graph.
    pub(super) graph: CommandGraph,
    pub(super) permission_catalog: PermissionCatalog,
    permission_metadata_catalog: PermissionMetadataCatalog,
    permission_context_catalog: PermissionContextCatalog,
}

impl CommandDispatcher {
    /// Creates a new command dispatcher with built-in commands.
    ///
    /// # Errors
    ///
    /// Returns an error when a built-in command registration is invalid.
    pub fn new() -> Result<Self, CommandRegistrationError> {
        Self::new_with_default_command_permissions(false)
    }

    /// Creates a new command dispatcher with built-in commands.
    ///
    /// # Errors
    ///
    /// Returns an error when a built-in command registration is invalid.
    pub fn new_with_default_command_permissions(
        require_default_command_permissions: bool,
    ) -> Result<Self, CommandRegistrationError> {
        let mut dispatcher = CommandDispatcher::new_empty();
        dispatcher.permission_catalog.insert(
            entity_selector_permission_key()?,
            PermissionCatalogSource::Command,
        );
        dispatcher.permission_catalog.insert(
            entity_selector_advanced_permission_key()?,
            PermissionCatalogSource::Command,
        );
        for registration in commands::registrations()? {
            dispatcher.register_command_with_defaults(
                registration,
                require_default_command_permissions,
            )?;
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

    #[cfg(test)]
    pub(super) fn register_command(
        &mut self,
        registration: CommandRegistration,
    ) -> Result<(), CommandRegistrationError> {
        self.register_command_with_defaults(registration, true)
    }

    pub(super) fn register_command_with_defaults(
        &mut self,
        registration: CommandRegistration,
        require_default_command_permissions: bool,
    ) -> Result<(), CommandRegistrationError> {
        let permission = registration.resolved_permission()?;
        let mut command_catalog = PermissionCatalog::new();
        command_catalog.insert(permission.key.clone(), PermissionCatalogSource::Command);
        let root = if permission.default_access && !require_default_command_permissions {
            registration
                .root
                .clone()
                .resolve_default_command_permissions(&permission.key, &mut command_catalog)?
        } else {
            registration
                .root
                .clone()
                .resolve_subcommand_permissions(&permission.key, &mut command_catalog)?
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

    pub(crate) fn parse_error_to_command_error(
        input: &str,
        error: CommandParseError,
    ) -> CommandError {
        let cursor = error.cursor();
        CommandError::parse(Self::parse_error_message(error.kind()), input, cursor)
    }

    pub(super) fn parse_error_invokes_result_callback(kind: &CommandParseErrorKind) -> bool {
        matches!(kind, CommandParseErrorKind::IncompleteCommand)
    }

    pub(super) fn parse_error_message(kind: &CommandParseErrorKind) -> TextComponent {
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
            CommandParseErrorKind::ExpectedInteger => TextComponent::plain("Expected integer"),
            CommandParseErrorKind::InvalidInteger(value) => translations::PARSING_INT_INVALID
                .message([TextComponent::from(value.clone())])
                .into(),
            CommandParseErrorKind::ExpectedLong => TextComponent::plain("Expected long"),
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
            CommandParseErrorKind::ExpectedFloat => TextComponent::plain("Expected float"),
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
            CommandParseErrorKind::ExpectedDouble => TextComponent::plain("Expected double"),
            CommandParseErrorKind::InvalidDouble(value) => {
                TextComponent::plain(format!("Invalid double '{value}'"))
            }
            CommandParseErrorKind::DoubleTooLow { value, min } => {
                TextComponent::plain(format!("Double {value} must not be less than {min}"))
            }
            CommandParseErrorKind::DoubleTooHigh { value, max } => {
                TextComponent::plain(format!("Double {value} must not be greater than {max}"))
            }
            CommandParseErrorKind::InvalidGameMode(value) => {
                translations::ARGUMENT_GAMEMODE_INVALID
                    .message([TextComponent::from(value.clone())])
                    .into()
            }
            CommandParseErrorKind::InvalidPlayer(_) => {
                TextComponent::from(&translations::ARGUMENT_ENTITY_NOTFOUND_PLAYER)
            }
            CommandParseErrorKind::TooManyPlayers => TextComponent::translated(TranslatedMessage {
                key: Cow::Borrowed("argument.player.toomany"),
                fallback: None,
                args: None,
            }),
            CommandParseErrorKind::InvalidEntity(_) => {
                TextComponent::from(&translations::ARGUMENT_ENTITY_NOTFOUND_ENTITY)
            }
            CommandParseErrorKind::TooManyEntities => {
                TextComponent::translated(TranslatedMessage {
                    key: Cow::Borrowed("argument.entity.toomany"),
                    fallback: None,
                    args: None,
                })
            }
            CommandParseErrorKind::EntitiesNotAllowedForPlayerArgument => {
                TextComponent::translated(TranslatedMessage {
                    key: Cow::Borrowed("argument.player.entities"),
                    fallback: None,
                    args: None,
                })
            }
            CommandParseErrorKind::EntitySelectorsNotAllowed => {
                TextComponent::plain("Selector syntax is not allowed for this command source")
            }
            CommandParseErrorKind::AdvancedEntitySelectorsNotAllowed => TextComponent::plain(
                "Advanced selector options are not allowed for this command source",
            ),
            CommandParseErrorKind::InvalidEntitySelector(value) => {
                TextComponent::plain(format!("Invalid entity selector: {value}"))
            }
            CommandParseErrorKind::UnsupportedEntitySelectorOption(value) => {
                TextComponent::plain(format!("Unsupported entity selector option: {value}"))
            }
            CommandParseErrorKind::InvalidItem(value) => translations::ARGUMENT_ITEM_ID_INVALID
                .message([TextComponent::from(value.clone())])
                .into(),
            CommandParseErrorKind::InvalidItemStack(value) => {
                TextComponent::plain(format!("Invalid item stack: {value}"))
            }
            CommandParseErrorKind::InvalidItemSlot(value) => {
                TextComponent::plain(format!("Invalid item slot '{value}'"))
            }
            CommandParseErrorKind::InvalidItemPredicate(value) => {
                TextComponent::plain(format!("Invalid item predicate: {value}"))
            }
            CommandParseErrorKind::InvalidLootPredicate(value) => {
                TextComponent::plain(format!("Invalid loot predicate: {value}"))
            }
            CommandParseErrorKind::InvalidCommandFunction(value) => {
                TextComponent::plain(format!("Invalid command function '{value}'"))
            }
            CommandParseErrorKind::InvalidWorld(value) => translations::ARGUMENT_DIMENSION_INVALID
                .message([TextComponent::from(value.clone())])
                .into(),
            CommandParseErrorKind::InvalidIdentifier(value) => {
                TextComponent::plain(format!("Invalid identifier '{value}'"))
            }
            CommandParseErrorKind::InvalidComponent(value) => {
                translations::ARGUMENT_COMPONENT_INVALID
                    .message([TextComponent::from(value.clone())])
                    .into()
            }
            CommandParseErrorKind::InvalidIntegerRange(value) => translations::PARSING_INT_INVALID
                .message([TextComponent::from(value.clone())])
                .into(),
            CommandParseErrorKind::SwappedIntegerRange => {
                TextComponent::from(&translations::ARGUMENT_RANGE_SWAPPED)
            }
            CommandParseErrorKind::InvalidDoubleRange(value) => {
                TextComponent::plain(format!("Invalid double range '{value}'"))
            }
            CommandParseErrorKind::SwappedDoubleRange => {
                TextComponent::from(&translations::ARGUMENT_RANGE_SWAPPED)
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
            CommandParseErrorKind::InvalidNbt(value) => {
                TextComponent::plain(format!("Invalid NBT: {value}"))
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
            CommandParseErrorKind::DynamicPermissionResolution(value) => {
                TextComponent::plain(format!("Invalid dynamic permission: {value}"))
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
