//! Target command argument parsers.

use std::sync::Arc;

use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use steel_utils::translations::{
    self, ARGUMENT_ENTITY_SELECTOR_ALL_PLAYERS, ARGUMENT_ENTITY_SELECTOR_NEAREST_PLAYER,
    ARGUMENT_ENTITY_SELECTOR_RANDOM_PLAYER, ARGUMENT_ENTITY_SELECTOR_SELF,
};
use text_components::TextComponent;
use uuid::Uuid;

use crate::{
    command::{
        CommandDispatcher,
        error::CommandError,
        graph::{
            CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
            CommandParseErrorKind, ParsedArgument, ParsedArguments, PermissionTarget,
        },
        parsers::selector::{
            EntitySelector, parse_entity_selector_argument, parse_player_selector,
            parse_player_selector_argument, selector_argument_suggestions,
        },
        reader::CommandReader,
        requirement::CommandInputContext,
    },
    entity::{Entity, SharedEntity},
    player::Player,
};

/// Parsed player target argument that resolves against the runtime command context.
#[derive(Clone, Debug)]
pub struct PlayerTargetArgumentValue {
    selector: EntitySelector,
    cursor: usize,
    input: String,
    single: bool,
}

impl PlayerTargetArgumentValue {
    fn new(selector: EntitySelector, cursor: usize, input: String, single: bool) -> Self {
        Self {
            selector,
            cursor,
            input,
            single,
        }
    }

    /// Returns the original selector text.
    #[must_use]
    pub fn raw(&self) -> &str {
        self.selector.raw()
    }

    /// Resolves this target against a runtime command context.
    ///
    /// # Errors
    ///
    /// Returns a structured command error when the selector cannot resolve.
    pub fn resolve_optional(
        &self,
        context: &dyn CommandInputContext,
    ) -> Result<Vec<Arc<Player>>, CommandError> {
        self.resolve_for_parse(context)
            .map_err(|error| CommandDispatcher::parse_error_to_command_error(&self.input, error))
    }

    /// Resolves this target and errors when no player matched.
    ///
    /// # Errors
    ///
    /// Returns a structured command error when the selector cannot resolve or no player matched.
    pub fn resolve_required(
        &self,
        context: &dyn CommandInputContext,
    ) -> Result<Vec<Arc<Player>>, CommandError> {
        let players = self.resolve_optional(context)?;
        if players.is_empty() {
            return Err(self.parse_error(CommandParseErrorKind::InvalidPlayer(
                self.selector.raw().to_owned(),
            )));
        }
        Ok(players)
    }

    fn resolve_for_parse(
        &self,
        context: &dyn CommandInputContext,
    ) -> Result<Vec<Arc<Player>>, CommandParseError> {
        let players = self.selector.find_players(context, self.cursor)?;
        if self.single && players.len() != 1 {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidPlayer(self.selector.raw().to_owned()),
                self.cursor,
            ));
        }
        Ok(players)
    }

    fn parse_error(&self, kind: CommandParseErrorKind) -> CommandError {
        CommandDispatcher::parse_error_to_command_error(
            &self.input,
            CommandParseError::new(kind, self.cursor),
        )
    }
}

/// Parsed entity target argument that resolves against the runtime command context.
#[derive(Clone, Debug)]
pub struct EntityTargetArgumentValue {
    selector: EntitySelector,
    cursor: usize,
    input: String,
    single: bool,
}

impl EntityTargetArgumentValue {
    pub(in crate::command::parsers) fn new(
        selector: EntitySelector,
        cursor: usize,
        input: String,
        single: bool,
    ) -> Self {
        Self {
            selector,
            cursor,
            input,
            single,
        }
    }

    /// Returns the original selector text.
    #[must_use]
    pub fn raw(&self) -> &str {
        self.selector.raw()
    }

    /// Resolves this target against a runtime command context.
    ///
    /// # Errors
    ///
    /// Returns a structured command error when the selector cannot resolve.
    pub fn resolve_optional(
        &self,
        context: &dyn CommandInputContext,
    ) -> Result<Vec<SharedEntity>, CommandError> {
        self.resolve_for_parse(context)
            .map_err(|error| CommandDispatcher::parse_error_to_command_error(&self.input, error))
    }

    /// Resolves this target and errors when no entity matched.
    ///
    /// # Errors
    ///
    /// Returns a structured command error when the selector cannot resolve or no entity matched.
    pub fn resolve_required(
        &self,
        context: &dyn CommandInputContext,
    ) -> Result<Vec<SharedEntity>, CommandError> {
        let entities = self.resolve_optional(context)?;
        if entities.is_empty() {
            return Err(self.parse_error(CommandParseErrorKind::InvalidEntity(
                self.selector.raw().to_owned(),
            )));
        }
        Ok(entities)
    }

    fn resolve_for_parse(
        &self,
        context: &dyn CommandInputContext,
    ) -> Result<Vec<SharedEntity>, CommandParseError> {
        let entities = self.selector.find_entities(context, self.cursor)?;
        if self.single && entities.len() != 1 {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidEntity(self.selector.raw().to_owned()),
                self.cursor,
            ));
        }
        Ok(entities)
    }

    fn parse_error(&self, kind: CommandParseErrorKind) -> CommandError {
        CommandDispatcher::parse_error_to_command_error(
            &self.input,
            CommandParseError::new(kind, self.cursor),
        )
    }
}

/// Player target argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerParser {
    one: bool,
}

impl PlayerParser {
    /// Creates a selector accepting multiple players.
    #[must_use]
    pub const fn multiple() -> Self {
        Self { one: false }
    }

    /// Creates a selector accepting one player.
    #[must_use]
    pub const fn one() -> Self {
        Self { one: true }
    }
}

impl CommandArgumentParser for PlayerParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let input = reader.input().to_owned();
        let (selector, cursor) = parse_player_selector_argument(reader, context, self.one)?;
        Ok(ParsedArgument::PlayerTargets(
            PlayerTargetArgumentValue::new(selector, cursor, input, self.one),
        ))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(
            ArgumentType::Entity {
                flags: 2 | u8::from(self.one),
            },
            Some(SuggestionType::AskServer),
        )
    }

    fn parsed_type(&self) -> &'static str {
        "players"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let mut suggestions = Vec::new();
        push_selector_suggestions(&mut suggestions, prefix, true, self.one, context);

        if let Some(server) = context.server() {
            let players = server.get_players();
            suggestions.extend(
                players
                    .iter()
                    .filter(|player| player.gameprofile.name.starts_with(prefix))
                    .map(|player| SuggestionEntry::new(player.gameprofile.name.clone())),
            );
        }

        suggestions
    }
}

/// Player target parser for permission-management commands.
#[derive(Clone, Copy, Debug, Default)]
pub struct PermissionTargetParser;

impl CommandArgumentParser for PermissionTargetParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        if reader.peek() == Some('@') {
            let players = parse_player_selector(reader, context, false)?;
            let targets = players
                .iter()
                .map(|player| PermissionTarget::online(player))
                .collect();
            return Ok(ParsedArgument::PermissionTargets(targets));
        }

        let value = reader.read_token()?;
        let Some(server) = context.server() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("server"),
                cursor,
            ));
        };

        let players = server.get_players();
        let uuid = Uuid::parse_str(&value).ok();
        let targets = if let Some(player) = players.into_iter().find(|player| {
            player.gameprofile.name.eq_ignore_ascii_case(&value)
                || uuid.is_some_and(|uuid| player.uuid() == uuid)
        }) {
            vec![PermissionTarget::online(&player)]
        } else {
            let known_players = server.known_players();
            let known = uuid
                .and_then(|uuid| known_players.by_uuid(uuid))
                .or_else(|| known_players.by_name(&value));
            known.map_or_else(
                || vec![PermissionTarget::unresolved(value)],
                |known| {
                    vec![PermissionTarget::offline(
                        known.uuid(),
                        known.last_known_name().to_owned(),
                    )]
                },
            )
        };

        Ok(ParsedArgument::PermissionTargets(targets))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(ArgumentType::GameProfile, Some(SuggestionType::AskServer))
    }

    fn parsed_type(&self) -> &'static str {
        "permission_targets"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let mut suggestions = vec![];
        push_selector_suggestions(&mut suggestions, prefix, true, false, context);

        if let Some(server) = context.server() {
            for player in server.get_players() {
                if player.gameprofile.name.starts_with(prefix) {
                    push_unique_suggestion(&mut suggestions, player.gameprofile.name.clone());
                }
            }
            for known in server.known_players().entries() {
                let name = known.last_known_name();
                if name.starts_with(prefix) {
                    push_unique_suggestion(&mut suggestions, name.to_owned());
                }
            }
        }

        suggestions
    }
}

fn push_unique_suggestion(suggestions: &mut Vec<SuggestionEntry>, text: String) {
    if suggestions
        .iter()
        .any(|suggestion| suggestion.text.eq_ignore_ascii_case(&text))
    {
        return;
    }
    suggestions.push(SuggestionEntry::new(text));
}

/// Living entity target argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct EntityParser {
    one: bool,
}

impl EntityParser {
    /// Creates a selector accepting multiple entities.
    #[must_use]
    pub const fn multiple() -> Self {
        Self { one: false }
    }

    /// Creates a selector accepting one entity.
    #[must_use]
    pub const fn one() -> Self {
        Self { one: true }
    }
}

impl CommandArgumentParser for EntityParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let input = reader.input().to_owned();
        let (selector, cursor) = parse_entity_selector_argument(reader, context, self.one)?;
        Ok(ParsedArgument::EntityTargets(
            EntityTargetArgumentValue::new(selector, cursor, input, self.one),
        ))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(
            ArgumentType::Entity {
                flags: u8::from(self.one),
            },
            Some(SuggestionType::AskServer),
        )
    }

    fn parsed_type(&self) -> &'static str {
        "entities"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let mut suggestions = Vec::new();
        push_selector_suggestions(&mut suggestions, prefix, false, self.one, context);

        if let Some(server) = context.server() {
            let players = server.get_players();
            suggestions.extend(
                players
                    .iter()
                    .filter(|player| player.gameprofile.name.starts_with(prefix))
                    .map(|player| SuggestionEntry::new(player.gameprofile.name.clone())),
            );
        }

        suggestions
    }
}

pub(crate) fn resolve_required_player_targets(
    arguments: &ParsedArguments,
    name: &str,
    context: &dyn CommandInputContext,
) -> Result<Vec<Arc<Player>>, CommandError> {
    if let Ok(targets) = arguments.get::<PlayerTargetArgumentValue>(name) {
        return targets.resolve_required(context);
    }
    let players = arguments
        .get::<Vec<Arc<Player>>>(name)
        .map_err(invalid_parsed_argument)?;
    if players.is_empty() {
        Err(no_players_found())
    } else {
        Ok(players)
    }
}

pub(crate) fn resolve_optional_player_targets(
    arguments: &ParsedArguments,
    name: &str,
    context: &dyn CommandInputContext,
) -> Result<Vec<Arc<Player>>, CommandError> {
    if let Ok(targets) = arguments.get::<PlayerTargetArgumentValue>(name) {
        return targets.resolve_optional(context);
    }
    arguments
        .get::<Vec<Arc<Player>>>(name)
        .map_err(invalid_parsed_argument)
}

pub(crate) fn resolve_required_entity_targets(
    arguments: &ParsedArguments,
    name: &str,
    context: &dyn CommandInputContext,
) -> Result<Vec<SharedEntity>, CommandError> {
    if let Ok(targets) = arguments.get::<EntityTargetArgumentValue>(name) {
        return targets.resolve_required(context);
    }
    let entities = arguments
        .get::<Vec<SharedEntity>>(name)
        .map_err(invalid_parsed_argument)?;
    if entities.is_empty() {
        Err(no_entities_found())
    } else {
        Ok(entities)
    }
}

pub(crate) fn resolve_optional_entity_targets(
    arguments: &ParsedArguments,
    name: &str,
    context: &dyn CommandInputContext,
) -> Result<Vec<SharedEntity>, CommandError> {
    if let Ok(targets) = arguments.get::<EntityTargetArgumentValue>(name) {
        return targets.resolve_optional(context);
    }
    arguments
        .get::<Vec<SharedEntity>>(name)
        .map_err(invalid_parsed_argument)
}

fn invalid_parsed_argument(error: crate::command::graph::ParsedArgumentError) -> CommandError {
    CommandError::InvalidConsumption(Some(format!("{error:?}")))
}

fn no_players_found() -> CommandError {
    CommandError::failure(TextComponent::from(
        &translations::ARGUMENT_ENTITY_NOTFOUND_PLAYER,
    ))
}

fn no_entities_found() -> CommandError {
    CommandError::failure(TextComponent::from(
        &translations::ARGUMENT_ENTITY_NOTFOUND_ENTITY,
    ))
}

fn push_selector_suggestions(
    suggestions: &mut Vec<SuggestionEntry>,
    prefix: &str,
    players_only: bool,
    single: bool,
    context: &dyn CommandInputContext,
) {
    for selector in selector_argument_suggestions(prefix, players_only, single, context) {
        match selector.as_str() {
            "@a" => suggestions.push(SuggestionEntry::with_tooltip(
                "@a",
                &ARGUMENT_ENTITY_SELECTOR_ALL_PLAYERS,
            )),
            "@p" => suggestions.push(SuggestionEntry::with_tooltip(
                "@p",
                &ARGUMENT_ENTITY_SELECTOR_NEAREST_PLAYER,
            )),
            "@r" => suggestions.push(SuggestionEntry::with_tooltip(
                "@r",
                &ARGUMENT_ENTITY_SELECTOR_RANDOM_PLAYER,
            )),
            "@s" => suggestions.push(SuggestionEntry::with_tooltip(
                "@s",
                &ARGUMENT_ENTITY_SELECTOR_SELF,
            )),
            "@e" | "@n" => suggestions.push(SuggestionEntry::new(selector)),
            _ => suggestions.push(SuggestionEntry::new(selector)),
        }
    }
}

#[cfg(test)]
mod tests {
    use steel_utils::translations;
    use text_components::content::Content;

    use crate::command::{
        error::CommandError,
        graph::{ParsedArgument, ParsedArguments},
        requirement::{CommandInputContext, CommandSourceKind, PermissionExpr, RequirementContext},
    };

    use super::{
        resolve_optional_entity_targets, resolve_optional_player_targets,
        resolve_required_entity_targets, resolve_required_player_targets,
    };

    struct TestContext;

    impl RequirementContext for TestContext {
        fn source_kind(&self) -> CommandSourceKind {
            CommandSourceKind::Console
        }

        fn has_permission(&self, _permission: &PermissionExpr) -> bool {
            false
        }
    }

    impl CommandInputContext for TestContext {}

    fn command_failed_translation_key(error: CommandError) -> String {
        let CommandError::CommandFailed(message) = error else {
            panic!("error should be a command failure");
        };
        let Content::Translate(message) = &message.content else {
            panic!("command failure should be translated");
        };
        message.key.to_string()
    }

    #[test]
    fn optional_player_targets_allow_empty_results() {
        let mut arguments = ParsedArguments::default();
        arguments.insert("targets", ParsedArgument::Players(Vec::new()));

        let Ok(targets) = resolve_optional_player_targets(&arguments, "targets", &TestContext)
        else {
            panic!("optional targets resolve");
        };

        assert!(targets.is_empty());
    }

    #[test]
    fn required_player_targets_reject_empty_results() {
        let mut arguments = ParsedArguments::default();
        arguments.insert("targets", ParsedArgument::Players(Vec::new()));

        let Err(error) = resolve_required_player_targets(&arguments, "targets", &TestContext)
        else {
            panic!("required targets reject empty results");
        };

        assert_eq!(
            command_failed_translation_key(error),
            translations::ARGUMENT_ENTITY_NOTFOUND_PLAYER.0
        );
    }

    #[test]
    fn optional_entity_targets_allow_empty_results() {
        let mut arguments = ParsedArguments::default();
        arguments.insert("targets", ParsedArgument::Entities(Vec::new()));

        let Ok(targets) = resolve_optional_entity_targets(&arguments, "targets", &TestContext)
        else {
            panic!("optional targets resolve");
        };

        assert!(targets.is_empty());
    }

    #[test]
    fn required_entity_targets_reject_empty_results() {
        let mut arguments = ParsedArguments::default();
        arguments.insert("targets", ParsedArgument::Entities(Vec::new()));

        let Err(error) = resolve_required_entity_targets(&arguments, "targets", &TestContext)
        else {
            panic!("required targets reject empty results");
        };

        assert_eq!(
            command_failed_translation_key(error),
            translations::ARGUMENT_ENTITY_NOTFOUND_ENTITY.0
        );
    }
}
