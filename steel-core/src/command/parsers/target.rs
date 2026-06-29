//! Target command argument parsers.

use std::sync::Arc;

use rand::seq::IteratorRandom;
use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use steel_utils::translations::{
    ARGUMENT_ENTITY_SELECTOR_ALL_PLAYERS, ARGUMENT_ENTITY_SELECTOR_NEAREST_PLAYER,
    ARGUMENT_ENTITY_SELECTOR_RANDOM_PLAYER, ARGUMENT_ENTITY_SELECTOR_SELF,
};
use uuid::Uuid;

use crate::{
    command::{
        graph::{
            CommandArgumentClientParser, CommandArgumentParser, CommandParseError,
            CommandParseErrorKind, ParsedArgument, ParsedArguments, PermissionTarget,
        },
        reader::CommandReader,
        requirement::CommandInputContext,
    },
    entity::{Entity, LivingEntity},
};

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
        let cursor = reader.absolute_cursor();
        let value = reader.read_token()?;
        let Some(server) = context.server() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("server"),
                cursor,
            ));
        };

        let players = server.get_players();
        let targets = match value.as_str() {
            "@a" if self.one => {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidPlayer(value),
                    cursor,
                ));
            }
            "@a" => players,
            "@p" => {
                let Some(position) = context.position() else {
                    return Err(CommandParseError::new(
                        CommandParseErrorKind::MissingCommandContext("position"),
                        cursor,
                    ));
                };

                let Some(first) = players.first() else {
                    return Ok(ParsedArgument::Players(Vec::new()));
                };
                let mut nearest = (f64::MAX, Arc::clone(first));
                for player in players {
                    let distance = player.position().distance_squared(position);
                    if distance < nearest.0 {
                        nearest = (distance, player);
                    }
                }
                vec![nearest.1]
            }
            "@r" => {
                let Some(player) = players.into_iter().choose(&mut rand::rng()) else {
                    return Ok(ParsedArgument::Players(Vec::new()));
                };
                vec![player]
            }
            "@s" => context
                .player()
                .map_or_else(Vec::new, |player| vec![Arc::clone(player)]),
            name => {
                let uuid = Uuid::parse_str(name).ok();
                let Some(player) = players.into_iter().find(|player| {
                    player.gameprofile.name == name
                        || uuid.is_some_and(|uuid| player.uuid() == uuid)
                }) else {
                    return Err(CommandParseError::new(
                        CommandParseErrorKind::InvalidPlayer(value),
                        cursor,
                    ));
                };
                vec![player]
            }
        };
        if self.one && targets.len() != 1 {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidPlayer(value),
                cursor,
            ));
        }

        Ok(ParsedArgument::Players(targets))
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
        if !self.one {
            suggestions.push(SuggestionEntry::with_tooltip(
                "@a",
                &ARGUMENT_ENTITY_SELECTOR_ALL_PLAYERS,
            ));
        }
        suggestions.extend([
            SuggestionEntry::with_tooltip("@p", &ARGUMENT_ENTITY_SELECTOR_NEAREST_PLAYER),
            SuggestionEntry::with_tooltip("@r", &ARGUMENT_ENTITY_SELECTOR_RANDOM_PLAYER),
            SuggestionEntry::with_tooltip("@s", &ARGUMENT_ENTITY_SELECTOR_SELF),
        ]);

        if let Some(server) = context.server() {
            let players = server.get_players();
            suggestions.extend(
                players
                    .iter()
                    .map(|player| SuggestionEntry::new(player.gameprofile.name.clone())),
            );
        }

        suggestions.retain(|suggestion| suggestion.text.starts_with(prefix));
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
        let value = reader.read_token()?;
        let Some(server) = context.server() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("server"),
                cursor,
            ));
        };

        let players = server.get_players();
        let targets = match value.as_str() {
            "@a" => players
                .into_iter()
                .map(|player| PermissionTarget::online(&player))
                .collect::<Vec<_>>(),
            "@p" => {
                let Some(position) = context.position() else {
                    return Err(CommandParseError::new(
                        CommandParseErrorKind::MissingCommandContext("position"),
                        cursor,
                    ));
                };

                let Some(nearest) = players.into_iter().min_by(|left, right| {
                    let left_distance = left.position().distance_squared(position);
                    let right_distance = right.position().distance_squared(position);
                    left_distance.total_cmp(&right_distance)
                }) else {
                    return Ok(ParsedArgument::PermissionTargets(Vec::new()));
                };
                vec![PermissionTarget::online(&nearest)]
            }
            "@r" => players
                .into_iter()
                .choose(&mut rand::rng())
                .map_or_else(Vec::new, |player| vec![PermissionTarget::online(&player)]),
            "@s" => context
                .player()
                .map_or_else(Vec::new, |player| vec![PermissionTarget::online(player)]),
            selector if selector.starts_with('@') => {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidPlayer(value),
                    cursor,
                ));
            }
            name => {
                let uuid = Uuid::parse_str(name).ok();
                if let Some(player) = players.into_iter().find(|player| {
                    player.gameprofile.name.eq_ignore_ascii_case(name)
                        || uuid.is_some_and(|uuid| player.uuid() == uuid)
                }) {
                    vec![PermissionTarget::online(&player)]
                } else {
                    let known_players = server.known_players();
                    let known = uuid
                        .and_then(|uuid| known_players.by_uuid(uuid))
                        .or_else(|| known_players.by_name(name));
                    known.map_or_else(
                        || vec![PermissionTarget::unresolved(name.to_owned())],
                        |known| {
                            vec![PermissionTarget::offline(
                                known.uuid(),
                                known.last_known_name().to_owned(),
                            )]
                        },
                    )
                }
            }
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
        let mut suggestions = vec![
            SuggestionEntry::with_tooltip("@a", &ARGUMENT_ENTITY_SELECTOR_ALL_PLAYERS),
            SuggestionEntry::with_tooltip("@p", &ARGUMENT_ENTITY_SELECTOR_NEAREST_PLAYER),
            SuggestionEntry::with_tooltip("@r", &ARGUMENT_ENTITY_SELECTOR_RANDOM_PLAYER),
            SuggestionEntry::with_tooltip("@s", &ARGUMENT_ENTITY_SELECTOR_SELF),
        ];

        if let Some(server) = context.server() {
            for player in server.get_players() {
                push_unique_suggestion(&mut suggestions, player.gameprofile.name.clone());
            }
            for known in server.known_players().entries() {
                push_unique_suggestion(&mut suggestions, known.last_known_name().to_owned());
            }
        }

        suggestions.retain(|suggestion| suggestion.text.starts_with(prefix));
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
        let cursor = reader.absolute_cursor();
        let value = reader.read_token()?;
        let Some(server) = context.server() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("server"),
                cursor,
            ));
        };

        let players = server.get_players();
        let targets = match value.as_str() {
            "@a" if self.one => {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidEntity(value),
                    cursor,
                ));
            }
            "@a" => players
                .into_iter()
                .map(|player| player as Arc<dyn LivingEntity + Send + Sync>)
                .collect(),
            "@p" => {
                let Some(position) = context.position() else {
                    return Err(CommandParseError::new(
                        CommandParseErrorKind::MissingCommandContext("position"),
                        cursor,
                    ));
                };

                let Some(first) = players.first() else {
                    return Ok(ParsedArgument::Entities(Vec::new()));
                };
                let mut nearest = (f64::MAX, Arc::clone(first));
                for player in players {
                    let distance = player.position().distance_squared(position);
                    if distance < nearest.0 {
                        nearest = (distance, player);
                    }
                }
                vec![nearest.1 as Arc<dyn LivingEntity + Send + Sync>]
            }
            "@r" => {
                let Some(player) = players.into_iter().choose(&mut rand::rng()) else {
                    return Ok(ParsedArgument::Entities(Vec::new()));
                };
                vec![player as Arc<dyn LivingEntity + Send + Sync>]
            }
            "@s" => context.player().map_or_else(Vec::new, |player| {
                vec![Arc::clone(player) as Arc<dyn LivingEntity + Send + Sync>]
            }),
            selector if selector.starts_with('@') => {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidEntity(value),
                    cursor,
                ));
            }
            name => {
                let uuid = Uuid::parse_str(name).ok();
                let Some(player) = players.into_iter().find(|player| {
                    player.gameprofile.name == name
                        || uuid.is_some_and(|uuid| player.uuid() == uuid)
                }) else {
                    return Err(CommandParseError::new(
                        CommandParseErrorKind::InvalidEntity(value),
                        cursor,
                    ));
                };
                vec![player as Arc<dyn LivingEntity + Send + Sync>]
            }
        };
        if self.one && targets.len() != 1 {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidEntity(value),
                cursor,
            ));
        }

        Ok(ParsedArgument::Entities(targets))
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
        "entities"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let mut suggestions = Vec::new();
        if !self.one {
            suggestions.push(SuggestionEntry::with_tooltip(
                "@a",
                &ARGUMENT_ENTITY_SELECTOR_ALL_PLAYERS,
            ));
        }
        suggestions.extend([
            SuggestionEntry::with_tooltip("@p", &ARGUMENT_ENTITY_SELECTOR_NEAREST_PLAYER),
            SuggestionEntry::with_tooltip("@r", &ARGUMENT_ENTITY_SELECTOR_RANDOM_PLAYER),
            SuggestionEntry::with_tooltip("@s", &ARGUMENT_ENTITY_SELECTOR_SELF),
        ]);

        if let Some(server) = context.server() {
            let players = server.get_players();
            suggestions.extend(
                players
                    .iter()
                    .map(|player| SuggestionEntry::new(player.gameprofile.name.clone())),
            );
        }

        suggestions.retain(|suggestion| suggestion.text.starts_with(prefix));
        suggestions
    }
}
