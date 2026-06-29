//! Graph-native command argument parsers.

use std::{collections::BTreeSet, f32::consts::PI, sync::Arc};

use glam::DVec3;
use rand::seq::IteratorRandom;
use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use steel_registry::{REGISTRY, RegistryExt, TaggedRegistryExt, entity_type::EntityTypeRef};
use steel_utils::{
    BlockPos, Identifier,
    translations::{
        ARGUMENT_ENTITY_SELECTOR_ALL_PLAYERS, ARGUMENT_ENTITY_SELECTOR_NEAREST_PLAYER,
        ARGUMENT_ENTITY_SELECTOR_RANDOM_PLAYER, ARGUMENT_ENTITY_SELECTOR_SELF,
    },
    types::GameType,
};
use text_components::TextComponent;
use uuid::Uuid;

use crate::{
    command::{
        context::EntityAnchor,
        graph::{
            CommandArgumentParser, CommandParseError, CommandParseErrorKind, ParsedArgument,
            ParsedArguments, PermissionTarget, StructureArgumentValue,
        },
        reader::{CommandReader, StringMode},
        requirement::CommandInputContext,
    },
    entity::{ENTITIES, Entity, LivingEntity},
    permission::{PermissionContextKey, PermissionKey, PermissionRuleExpression},
};

/// Game mode argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct GameModeParser;

impl CommandArgumentParser for GameModeParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let value = reader.read_string(StringMode::SingleWord)?;

        let game_mode = match value.to_lowercase().as_str() {
            "survival" | "0" => GameType::Survival,
            "creative" | "1" => GameType::Creative,
            "adventure" | "2" => GameType::Adventure,
            "spectator" | "3" => GameType::Spectator,
            _ => {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidGameMode(value),
                    cursor,
                ));
            }
        };

        Ok(ParsedArgument::GameMode(game_mode))
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (ArgumentType::Gamemode, None)
    }

    fn parsed_type(&self) -> &'static str {
        "gamemode"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        ["survival", "creative", "adventure", "spectator"]
            .into_iter()
            .filter(|suggestion| suggestion.starts_with(prefix))
            .map(SuggestionEntry::new)
            .collect()
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
        let cursor = reader.absolute_cursor();
        let value = reader.read_string(StringMode::SingleWord)?;
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

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (
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
        let value = reader.read_string(StringMode::SingleWord)?;
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

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (ArgumentType::GameProfile, Some(SuggestionType::AskServer))
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

/// Permission key argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct PermissionKeyParser;

impl CommandArgumentParser for PermissionKeyParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let value = reader.read_string(StringMode::SingleWord)?;
        let permission = PermissionKey::parse(value.clone()).map_err(|_| {
            CommandParseError::new(CommandParseErrorKind::InvalidPermissionKey(value), cursor)
        })?;

        Ok(ParsedArgument::PermissionKey(permission))
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (
            ArgumentType::String {
                behavior: steel_protocol::packets::game::ArgumentStringTypeBehavior::GreedyPhrase,
            },
            Some(SuggestionType::AskServer),
        )
    }

    fn parsed_type(&self) -> &'static str {
        "permission_key"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let Some(catalog) = context.permission_catalog() else {
            return Vec::new();
        };

        catalog
            .suggestions(prefix)
            .into_iter()
            .map(SuggestionEntry::new)
            .collect()
    }
}

/// Permission rule expression argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct PermissionRuleExpressionParser;

impl CommandArgumentParser for PermissionRuleExpressionParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let value = reader.read_string(StringMode::SingleWord)?;
        let expression = PermissionRuleExpression::parse(value).map_err(|error| {
            CommandParseError::new(
                CommandParseErrorKind::InvalidPermissionExpression(error.to_string()),
                cursor,
            )
        })?;

        Ok(ParsedArgument::PermissionRuleExpression(expression))
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (
            ArgumentType::String {
                behavior: steel_protocol::packets::game::ArgumentStringTypeBehavior::GreedyPhrase,
            },
            Some(SuggestionType::AskServer),
        )
    }

    fn parsed_type(&self) -> &'static str {
        "permission_rule_expression"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        permission_rule_expression_suggestions(prefix, context)
            .into_iter()
            .map(SuggestionEntry::new)
            .collect()
    }
}

fn permission_rule_expression_suggestions(
    prefix: &str,
    context: &dyn CommandInputContext,
) -> Vec<String> {
    let Some((permission_key, context_prefix)) = prefix.split_once('{') else {
        let Some(catalog) = context.permission_catalog() else {
            return Vec::new();
        };
        return catalog.suggestions(prefix);
    };
    if context_prefix.contains('}') || PermissionKey::parse(permission_key).is_err() {
        return Vec::new();
    }

    let (completed_entries, current_entry) = context_prefix
        .rsplit_once(',')
        .map_or(("", context_prefix), |(completed, current)| {
            (&context_prefix[..completed.len() + 1], current)
        });
    let expression_prefix = format!("{permission_key}{{{completed_entries}");
    let completed_keys = completed_permission_rule_context_keys(completed_entries);

    let Some((context_key, value_prefix)) = current_entry.split_once('=') else {
        return permission_rule_context_key_suggestions(
            &expression_prefix,
            current_entry,
            &completed_keys,
            context,
        );
    };
    if completed_keys.contains(context_key) {
        return Vec::new();
    }

    permission_rule_context_value_suggestions(
        &expression_prefix,
        context_key,
        value_prefix,
        context,
    )
}

fn completed_permission_rule_context_keys(completed_entries: &str) -> BTreeSet<String> {
    completed_entries
        .trim_end_matches(',')
        .split(',')
        .filter_map(|entry| {
            let (key, _) = entry.split_once('=')?;
            (!key.is_empty()).then(|| key.to_owned())
        })
        .collect()
}

fn permission_rule_context_key_suggestions(
    expression_prefix: &str,
    key_prefix: &str,
    completed_keys: &BTreeSet<String>,
    context: &dyn CommandInputContext,
) -> Vec<String> {
    let mut suggestions = ["domain", "world"]
        .into_iter()
        .filter(|key| !completed_keys.contains(*key))
        .filter(|key| key.starts_with(key_prefix))
        .map(|key| format!("{expression_prefix}{key}="))
        .collect::<Vec<_>>();

    if let Some(catalog) = context.permission_context_catalog() {
        suggestions.extend(
            catalog
                .key_suggestions(key_prefix)
                .into_iter()
                .filter(|key| !completed_keys.contains(key))
                .map(|key| format!("{expression_prefix}{key}=")),
        );
    }
    suggestions.sort();
    suggestions.dedup();
    suggestions
}

fn permission_rule_context_value_suggestions(
    expression_prefix: &str,
    context_key: &str,
    value_prefix: &str,
    context: &dyn CommandInputContext,
) -> Vec<String> {
    match context_key {
        "domain" => permission_rule_domain_value_suggestions(
            expression_prefix,
            context_key,
            value_prefix,
            context,
        ),
        "world" => permission_rule_world_value_suggestions(
            expression_prefix,
            context_key,
            value_prefix,
            context,
        ),
        custom_key => permission_rule_custom_context_value_suggestions(
            expression_prefix,
            custom_key,
            value_prefix,
            context,
        ),
    }
}

fn permission_rule_domain_value_suggestions(
    expression_prefix: &str,
    context_key: &str,
    value_prefix: &str,
    context: &dyn CommandInputContext,
) -> Vec<String> {
    let Some(server) = context.server() else {
        return Vec::new();
    };

    server
        .worlds
        .domain_names()
        .filter(|domain| domain.starts_with(value_prefix))
        .map(|domain| format!("{expression_prefix}{context_key}={domain}}}"))
        .collect()
}

fn permission_rule_world_value_suggestions(
    expression_prefix: &str,
    context_key: &str,
    value_prefix: &str,
    context: &dyn CommandInputContext,
) -> Vec<String> {
    let Some(server) = context.server() else {
        return Vec::new();
    };

    server
        .worlds
        .keys()
        .map(ToString::to_string)
        .filter(|world| world.starts_with(value_prefix))
        .map(|world| format!("{expression_prefix}{context_key}={world}}}"))
        .collect()
}

fn permission_rule_custom_context_value_suggestions(
    expression_prefix: &str,
    context_key: &str,
    value_prefix: &str,
    context: &dyn CommandInputContext,
) -> Vec<String> {
    let Some(catalog) = context.permission_context_catalog() else {
        return Vec::new();
    };
    let Ok(key) = PermissionContextKey::parse(context_key.to_owned()) else {
        return Vec::new();
    };

    catalog
        .value_suggestions(&key, value_prefix)
        .into_iter()
        .map(|value| format!("{expression_prefix}{context_key}={value}}}"))
        .collect()
}

/// Permission group name argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct PermissionGroupParser;

impl CommandArgumentParser for PermissionGroupParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let value = reader.read_string(StringMode::SingleWord)?;
        let Some(server) = context.server() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("server"),
                cursor,
            ));
        };
        if !server.permission_groups.contains_group(&value) {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidPermissionGroup(value),
                cursor,
            ));
        }

        Ok(ParsedArgument::String(value))
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (
            ArgumentType::String {
                behavior: steel_protocol::packets::game::ArgumentStringTypeBehavior::SingleWord,
            },
            Some(SuggestionType::AskServer),
        )
    }

    fn parsed_type(&self) -> &'static str {
        "string"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let Some(server) = context.server() else {
            return Vec::new();
        };

        server
            .permission_groups
            .group_names()
            .into_iter()
            .filter(|group| group.starts_with(prefix))
            .map(SuggestionEntry::new)
            .collect()
    }
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
        let value = reader.read_string(StringMode::SingleWord)?;
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

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (
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

/// Summonable entity type argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct EntitySummonParser;

impl CommandArgumentParser for EntitySummonParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_string(StringMode::SingleWord)?;
        let Some(entity_type) = resolve_summon_entity_type(&raw) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidEntityType(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::EntityType(entity_type))
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (
            ArgumentType::Resource {
                identifier: "minecraft:entity_type",
            },
            Some(SuggestionType::SummonableEntities),
        )
    }

    fn parsed_type(&self) -> &'static str {
        "entity_type"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let stripped_prefix = prefix.strip_prefix("minecraft:").unwrap_or(prefix);
        REGISTRY
            .entity_types
            .iter()
            .filter(|(_, entity_type)| can_summon_entity_type(entity_type))
            .map(|(_, entity_type)| SuggestionEntry::new(entity_type.key.to_string()))
            .filter(|suggestion| {
                suggestion
                    .text
                    .strip_prefix("minecraft:")
                    .unwrap_or(&suggestion.text)
                    .starts_with(stripped_prefix)
            })
            .collect()
    }
}

fn parse_resource_identifier(input: &str) -> Option<Identifier> {
    let (namespace, path) = input.split_once(':').map_or(
        (Identifier::VANILLA_NAMESPACE, input),
        |(namespace, path)| (namespace, path),
    );

    Identifier::validate(namespace, path)
        .then(|| Identifier::new(namespace.to_owned(), path.to_owned()))
}

fn resolve_summon_entity_type(input: &str) -> Option<EntityTypeRef> {
    let key = parse_resource_identifier(input)?;
    REGISTRY
        .entity_types
        .by_key(&key)
        .filter(|entity_type| can_summon_entity_type(entity_type))
}

fn can_summon_entity_type(entity_type: EntityTypeRef) -> bool {
    entity_type.summonable
        && ENTITIES
            .get()
            .is_some_and(|registry| registry.has_factory(entity_type))
}

/// Item stack argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct ItemParser;

impl CommandArgumentParser for ItemParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_string(StringMode::SingleWord)?;
        let key = raw.strip_prefix("minecraft:").unwrap_or(&raw).to_owned();

        let Some(item) = REGISTRY.items.by_key(&Identifier::vanilla(key)) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidItem(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::Item(item))
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (ArgumentType::ItemStack, Some(SuggestionType::AskServer))
    }

    fn parsed_type(&self) -> &'static str {
        "item"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let stripped_prefix = prefix.strip_prefix("minecraft:").unwrap_or(prefix);
        REGISTRY
            .items
            .iter()
            .map(|(_, item)| SuggestionEntry::new(item.key.to_string()))
            .filter(|suggestion| {
                suggestion
                    .text
                    .strip_prefix("minecraft:")
                    .unwrap_or(&suggestion.text)
                    .starts_with(stripped_prefix)
            })
            .collect()
    }
}

/// Enchantment argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct EnchantmentParser;

impl CommandArgumentParser for EnchantmentParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_string(StringMode::SingleWord)?;
        let key = raw.strip_prefix("minecraft:").unwrap_or(&raw).to_owned();

        let Some(enchantment) = REGISTRY.enchantments.by_key(&Identifier::vanilla(key)) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidEnchantment(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::Enchantment(enchantment))
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (
            ArgumentType::Resource {
                identifier: "minecraft:enchantment",
            },
            Some(SuggestionType::AskServer),
        )
    }

    fn parsed_type(&self) -> &'static str {
        "enchantment"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let stripped_prefix = prefix.strip_prefix("minecraft:").unwrap_or(prefix);
        REGISTRY
            .enchantments
            .iter()
            .map(|(_, enchantment)| SuggestionEntry::new(enchantment.key.to_string()))
            .filter(|suggestion| {
                suggestion
                    .text
                    .strip_prefix("minecraft:")
                    .unwrap_or(&suggestion.text)
                    .starts_with(stripped_prefix)
            })
            .collect()
    }
}

/// Structure or structure tag argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct StructureParser;

impl CommandArgumentParser for StructureParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_string(StringMode::SingleWord)?;

        if let Some(tag) = raw.strip_prefix('#') {
            let Some(key) = parse_resource_identifier(tag) else {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidStructure(raw),
                    cursor,
                ));
            };
            let Some(structures) = REGISTRY.structures.get_tag(&key) else {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidStructure(raw),
                    cursor,
                ));
            };
            if structures.is_empty() {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidStructure(raw),
                    cursor,
                ));
            }
            return Ok(ParsedArgument::Structure(StructureArgumentValue::Tag {
                key,
                structures,
            }));
        }

        let Some(key) = parse_resource_identifier(&raw) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidStructure(raw),
                cursor,
            ));
        };

        let Some(structure) = REGISTRY.structures.by_key(&key) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidStructure(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::Structure(
            StructureArgumentValue::Structure(structure),
        ))
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (
            ArgumentType::ResourceOrTagKey {
                identifier: "minecraft:worldgen/structure",
            },
            Some(SuggestionType::AskServer),
        )
    }

    fn parsed_type(&self) -> &'static str {
        "structure"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        if prefix.starts_with('#') {
            let stripped_prefix = prefix
                .strip_prefix("#minecraft:")
                .or_else(|| prefix.strip_prefix('#'))
                .unwrap_or(prefix);
            return REGISTRY
                .structures
                .tag_keys()
                .filter_map(|key| {
                    let key = key.to_string();
                    let text = key.strip_prefix("minecraft:").unwrap_or(&key);
                    text.starts_with(stripped_prefix)
                        .then(|| SuggestionEntry::new(format!("#{key}")))
                })
                .collect();
        }

        let stripped_prefix = prefix.strip_prefix("minecraft:").unwrap_or(prefix);
        let mut suggestions = Vec::new();
        suggestions.extend(
            REGISTRY
                .structures
                .iter()
                .map(|(_, structure)| SuggestionEntry::new(structure.key.to_string()))
                .filter(|suggestion| {
                    suggestion
                        .text
                        .strip_prefix("minecraft:")
                        .unwrap_or(&suggestion.text)
                        .starts_with(stripped_prefix)
                }),
        );
        suggestions.extend(
            REGISTRY
                .structures
                .tag_keys()
                .map(|key| SuggestionEntry::new(format!("#{key}")))
                .filter(|suggestion| {
                    suggestion
                        .text
                        .strip_prefix("#minecraft:")
                        .unwrap_or(&suggestion.text)
                        .starts_with(stripped_prefix)
                }),
        );
        suggestions
    }
}

/// Configured domain name argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct DomainParser;

impl CommandArgumentParser for DomainParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let domain = reader.read_string(StringMode::SingleWord)?;
        let Some(server) = context.server() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("server"),
                cursor,
            ));
        };

        if !server.worlds.has_domain(&domain) {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidDomain(domain),
                cursor,
            ));
        }

        Ok(ParsedArgument::String(domain))
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (
            ArgumentType::ResourceLocation,
            Some(SuggestionType::AskServer),
        )
    }

    fn parsed_type(&self) -> &'static str {
        "string"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let Some(server) = context.server() else {
            return Vec::new();
        };

        server
            .worlds
            .domain_names()
            .filter(|domain| domain.starts_with(prefix))
            .map(SuggestionEntry::new)
            .collect()
    }
}

/// Loaded world argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct WorldParser;

impl CommandArgumentParser for WorldParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_string(StringMode::SingleWord)?;
        let Some(server) = context.server() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("server"),
                cursor,
            ));
        };

        if let Some(world) = raw
            .parse::<Identifier>()
            .ok()
            .and_then(|key| server.worlds.get(&key).cloned())
        {
            return Ok(ParsedArgument::World(world));
        }

        let Some(current_world) = context.world() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("world"),
                cursor,
            ));
        };

        let key = Identifier::new(current_world.domain().to_owned(), raw.clone());
        let Some(world) = server.worlds.get(&key).cloned() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidWorld(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::World(world))
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (ArgumentType::Dimension, Some(SuggestionType::AskServer))
    }

    fn parsed_type(&self) -> &'static str {
        "world"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let (Some(server), Some(current_world)) = (context.server(), context.world()) else {
            return Vec::new();
        };
        let current_domain = current_world.domain();

        let mut suggestions = server
            .worlds
            .keys()
            .map(|id| SuggestionEntry::new(id.to_string()))
            .collect::<Vec<_>>();

        for id in server.worlds.keys() {
            if id.namespace.as_ref() == current_domain {
                let path = id.path.as_ref();
                if !suggestions.iter().any(|suggestion| suggestion.text == path) {
                    suggestions.push(SuggestionEntry::new(path));
                }
            }
        }

        suggestions.retain(|suggestion| suggestion.text.starts_with(prefix));
        suggestions
    }
}

/// 3D position argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct Vec3Parser;

impl CommandArgumentParser for Vec3Parser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let x = reader.read_string(StringMode::SingleWord)?;
        reader.expect_whitespace()?;
        let y = reader.read_string(StringMode::SingleWord)?;
        reader.expect_whitespace()?;
        let z = reader.read_string(StringMode::SingleWord)?;
        let raw = format!("{x} {y} {z}");

        if x.starts_with('^') {
            let Some(pos) = parse_local_coordinates((&x, &y, &z), context) else {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidVec3(raw),
                    cursor,
                ));
            };

            return Ok(ParsedArgument::Vec3(pos));
        }

        let Some(origin) = context.position() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("position"),
                cursor,
            ));
        };
        let Some(x) = parse_vec3_coordinate::<false>(&x, origin.x) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidVec3(raw),
                cursor,
            ));
        };
        let Some(y) = parse_vec3_coordinate::<true>(&y, origin.y) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidVec3(raw),
                cursor,
            ));
        };
        let Some(z) = parse_vec3_coordinate::<false>(&z, origin.z) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidVec3(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::Vec3(DVec3::new(x, y, z)))
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (ArgumentType::Vec3, None)
    }

    fn parsed_type(&self) -> &'static str {
        "vec3"
    }
}

/// Block position argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct BlockPosParser;

impl CommandArgumentParser for BlockPosParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let x = reader.read_string(StringMode::SingleWord)?;
        reader.expect_whitespace()?;
        let y = reader.read_string(StringMode::SingleWord)?;
        reader.expect_whitespace()?;
        let z = reader.read_string(StringMode::SingleWord)?;
        let raw = format!("{x} {y} {z}");

        if x.starts_with('^') {
            let Some(pos) = parse_local_coordinates((&x, &y, &z), context) else {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidBlockPos(raw),
                    cursor,
                ));
            };

            return Ok(ParsedArgument::BlockPos(BlockPos::containing(
                pos.x, pos.y, pos.z,
            )));
        }

        let Some(origin) = context.position() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::MissingCommandContext("position"),
                cursor,
            ));
        };
        let Some(x) = parse_block_coordinate(&x, origin.x) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidBlockPos(raw),
                cursor,
            ));
        };
        let Some(y) = parse_block_coordinate(&y, origin.y) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidBlockPos(raw),
                cursor,
            ));
        };
        let Some(z) = parse_block_coordinate(&z, origin.z) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidBlockPos(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::BlockPos(BlockPos::containing(x, y, z)))
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (ArgumentType::BlockPos, None)
    }

    fn parsed_type(&self) -> &'static str {
        "block_pos"
    }
}

/// Rotation argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct RotationParser;

impl CommandArgumentParser for RotationParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let yaw = reader.read_string(StringMode::SingleWord)?;
        reader.expect_whitespace()?;
        let pitch = reader.read_string(StringMode::SingleWord)?;
        let raw = format!("{yaw} {pitch}");

        let (origin_yaw, origin_pitch) = context.rotation().unwrap_or((0.0, 0.0));
        let Some(yaw) = parse_rotation_coordinate(&yaw, origin_yaw) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidRotation(raw),
                cursor,
            ));
        };
        let Some(pitch) = parse_rotation_coordinate(&pitch, origin_pitch) else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidRotation(raw),
                cursor,
            ));
        };

        Ok(ParsedArgument::Rotation(normalize_rotation((yaw, pitch))))
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (ArgumentType::Rotation, None)
    }

    fn parsed_type(&self) -> &'static str {
        "rotation"
    }
}

fn parse_block_coordinate(value: &str, origin: f64) -> Option<f64> {
    if value.starts_with('^') {
        return None;
    }

    if let Some(offset) = value.strip_prefix('~') {
        if offset.is_empty() {
            Some(origin)
        } else {
            Some(origin + offset.parse::<f64>().ok()?)
        }
    } else {
        Some(f64::from(value.parse::<i32>().ok()?))
    }
}

fn parse_vec3_coordinate<const IS_Y: bool>(value: &str, origin: f64) -> Option<f64> {
    if let Some(offset) = value.strip_prefix('~') {
        let offset = if offset.is_empty() {
            0.0
        } else {
            offset.parse().ok()?
        };
        return Some(origin + offset);
    }

    let mut parsed = value.parse::<f64>().ok()?;
    if !IS_Y && !value.contains('.') {
        parsed += 0.5;
    }

    Some(parsed)
}

fn parse_local_coordinates(
    coordinates: (&str, &str, &str),
    context: &dyn CommandInputContext,
) -> Option<DVec3> {
    let (left, up, forwards) = parse_local_coordinate_triplet(coordinates)?;
    let source = anchor_position(context)?;
    let rotation = context.rotation().unwrap_or((0.0, 0.0));

    Some(local_coordinates_to_anchor_position(
        source, rotation, left, up, forwards,
    ))
}

fn parse_local_coordinate_triplet(coordinates: (&str, &str, &str)) -> Option<(f64, f64, f64)> {
    let left = parse_local_coordinate(coordinates.0)?;
    let up = parse_local_coordinate(coordinates.1)?;
    let forwards = parse_local_coordinate(coordinates.2)?;
    Some((left, up, forwards))
}

fn parse_local_coordinate(value: &str) -> Option<f64> {
    let offset = value.strip_prefix('^')?;
    if offset.is_empty() {
        Some(0.0)
    } else {
        offset.parse::<f64>().ok()
    }
}

fn local_coordinates_to_anchor_position(
    source: DVec3,
    rotation: (f32, f32),
    left: f64,
    up: f64,
    forwards: f64,
) -> DVec3 {
    let (yaw, pitch) = rotation;
    let y_cos = ((yaw + 90.0) * PI / 180.0).cos();
    let y_sin = ((yaw + 90.0) * PI / 180.0).sin();
    let x_cos = (-pitch * PI / 180.0).cos();
    let x_sin = (-pitch * PI / 180.0).sin();
    let x_cos_up = ((-pitch + 90.0) * PI / 180.0).cos();
    let x_sin_up = ((-pitch + 90.0) * PI / 180.0).sin();
    let forwards_axis = DVec3::new(
        f64::from(y_cos * x_cos),
        f64::from(x_sin),
        f64::from(y_sin * x_cos),
    );
    let up_axis = DVec3::new(
        f64::from(y_cos * x_cos_up),
        f64::from(x_sin_up),
        f64::from(y_sin * x_cos_up),
    );
    let left_axis = -forwards_axis.cross(up_axis);

    source + left_axis * left + up_axis * up + forwards_axis * forwards
}

fn anchor_position(context: &dyn CommandInputContext) -> Option<DVec3> {
    let position = context.position()?;
    if matches!(context.anchor(), EntityAnchor::Eyes)
        && let Some(player) = context.player()
    {
        return Some(DVec3::new(position.x, player.get_eye_y(), position.z));
    }

    Some(position)
}

fn parse_rotation_coordinate(value: &str, origin: f32) -> Option<f32> {
    if value.starts_with('^') {
        return None;
    }

    if let Some(offset) = value.strip_prefix('~') {
        if offset.is_empty() {
            Some(origin)
        } else {
            Some(origin + offset.parse::<f32>().ok()?)
        }
    } else {
        value.parse::<f32>().ok()
    }
}

fn normalize_rotation((mut yaw, mut pitch): (f32, f32)) -> (f32, f32) {
    yaw = yaw.rem_euclid(360.0);
    if yaw >= 180.0 {
        yaw -= 360.0;
    }
    pitch = pitch.rem_euclid(360.0);
    if pitch >= 180.0 {
        pitch -= 360.0;
    }

    (yaw, pitch)
}

/// Text component argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct ComponentParser;

impl CommandArgumentParser for ComponentParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let raw = reader.read_string(StringMode::GreedyPhrase)?;
        let component = TextComponent::from_snbt(&raw).map_err(|error| {
            CommandParseError::new(
                CommandParseErrorKind::InvalidComponent(error.to_string()),
                cursor,
            )
        })?;

        Ok(ParsedArgument::Component(Box::new(component)))
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (ArgumentType::Component, None)
    }

    fn parsed_type(&self) -> &'static str {
        "component"
    }
}

/// Time argument parser.
#[derive(Clone, Copy, Debug, Default)]
pub struct TimeParser;

impl CommandArgumentParser for TimeParser {
    fn parse(
        &self,
        reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        let cursor = reader.absolute_cursor();
        let value = reader.read_string(StringMode::SingleWord)?;

        let (number, unit) = value
            .find(char::is_alphabetic)
            .map_or((value.as_str(), "t"), |pos| (&value[..pos], &value[pos..]));

        let Ok(number) = number.parse::<f32>() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidTime(value),
                cursor,
            ));
        };

        if number < 0.0 {
            return Err(CommandParseError::new(
                CommandParseErrorKind::InvalidTime(value),
                cursor,
            ));
        }

        let ticks = match unit {
            "d" => number * 24_000.0,
            "s" => number * 20.0,
            "t" => number,
            _ => {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::InvalidTime(value),
                    cursor,
                ));
            }
        };

        Ok(ParsedArgument::I32(ticks.round() as i32))
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (ArgumentType::Time { min: 0 }, None)
    }

    fn parsed_type(&self) -> &'static str {
        "i32"
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        _context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let has_unit = prefix.chars().any(char::is_alphabetic);
        if prefix.is_empty() || has_unit {
            return Vec::new();
        }

        ["d", "s", "t"]
            .into_iter()
            .map(|unit| SuggestionEntry::new(format!("{prefix}{unit}")))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec3;
    use steel_protocol::packets::game::SuggestionType;
    use steel_registry::{
        test_support::init_test_registry, vanilla_enchantments, vanilla_entities, vanilla_items,
    };

    use crate::{
        command::{
            graph::{
                CommandArgumentParser, CommandParseErrorKind, ParsedArgument, ParsedArguments,
            },
            parsers::{
                BlockPosParser, ComponentParser, DomainParser, EnchantmentParser, EntityParser,
                EntitySummonParser, GameModeParser, ItemParser, PermissionKeyParser,
                PermissionRuleExpressionParser, PermissionTargetParser, PlayerParser,
                RotationParser, StructureParser, TimeParser, Vec3Parser, WorldParser,
            },
            reader::CommandReader,
            requirement::{
                CommandInputContext, CommandSourceKind, PermissionExpr, RequirementContext,
            },
        },
        entity::init_test_entities,
        permission::{
            PermissionCatalog, PermissionCatalogSource, PermissionContextCatalog,
            PermissionContextCatalogSource, PermissionContextKey, PermissionKey,
        },
    };
    use steel_utils::types::GameType;

    struct TestContext;

    impl RequirementContext for TestContext {
        fn source_kind(&self) -> CommandSourceKind {
            CommandSourceKind::Player
        }

        fn has_permission(&self, _permission: &PermissionExpr) -> bool {
            false
        }
    }

    impl CommandInputContext for TestContext {}

    struct CatalogContext {
        catalog: PermissionCatalog,
        context_catalog: PermissionContextCatalog,
    }

    impl RequirementContext for CatalogContext {
        fn source_kind(&self) -> CommandSourceKind {
            CommandSourceKind::Player
        }

        fn has_permission(&self, _permission: &PermissionExpr) -> bool {
            false
        }
    }

    impl CommandInputContext for CatalogContext {
        fn permission_catalog(&self) -> Option<&PermissionCatalog> {
            Some(&self.catalog)
        }

        fn permission_context_catalog(&self) -> Option<&PermissionContextCatalog> {
            Some(&self.context_catalog)
        }
    }

    struct PositionedContext;

    impl RequirementContext for PositionedContext {
        fn source_kind(&self) -> CommandSourceKind {
            CommandSourceKind::Player
        }

        fn has_permission(&self, _permission: &PermissionExpr) -> bool {
            false
        }
    }

    impl CommandInputContext for PositionedContext {
        fn position(&self) -> Option<DVec3> {
            Some(DVec3::new(10.0, 20.0, 30.0))
        }
    }

    #[test]
    fn gamemode_parser_accepts_names_and_numeric_aliases() {
        let parser = GameModeParser;
        let context = TestContext;

        let mut reader = CommandReader::new("creative");
        let value = parser
            .parse(&mut reader, &context)
            .expect("game mode parses");
        assert!(matches!(
            value,
            ParsedArgument::GameMode(GameType::Creative)
        ));

        let mut reader = CommandReader::new("3");
        let value = parser
            .parse(&mut reader, &context)
            .expect("game mode parses");
        assert!(matches!(
            value,
            ParsedArgument::GameMode(GameType::Spectator)
        ));
    }

    #[test]
    fn gamemode_parser_suggests_matching_modes() {
        let suggestions = GameModeParser.suggest("s", &ParsedArguments::default(), &TestContext);
        let texts = suggestions
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();

        assert_eq!(texts, vec!["survival", "spectator"]);
    }

    #[test]
    fn permission_key_parser_validates_permission_keys() {
        let mut reader = CommandReader::new("steel.command.steelperms.*");
        let value = PermissionKeyParser
            .parse(&mut reader, &TestContext)
            .expect("permission key parses");
        assert!(
            matches!(value, ParsedArgument::PermissionKey(permission) if permission.as_str() == "steel.command.steelperms.*")
        );

        let mut reader = CommandReader::new("steel.*.steelperms");
        let error = PermissionKeyParser
            .parse(&mut reader, &TestContext)
            .expect_err("mid wildcard should be rejected");
        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::InvalidPermissionKey(value) if value == "steel.*.steelperms"
        ));
    }

    #[test]
    fn permission_management_parsers_request_server_suggestions() {
        assert!(matches!(
            PermissionKeyParser.usage().0,
            steel_protocol::packets::game::ArgumentType::String {
                behavior: steel_protocol::packets::game::ArgumentStringTypeBehavior::GreedyPhrase
            }
        ));
        assert!(matches!(
            PermissionKeyParser.usage().1,
            Some(SuggestionType::AskServer)
        ));
        assert!(matches!(
            PermissionRuleExpressionParser.usage().0,
            steel_protocol::packets::game::ArgumentType::String {
                behavior: steel_protocol::packets::game::ArgumentStringTypeBehavior::GreedyPhrase
            }
        ));
        assert!(matches!(
            PermissionRuleExpressionParser.usage().1,
            Some(SuggestionType::AskServer)
        ));
        assert!(matches!(
            super::PermissionGroupParser.usage().1,
            Some(SuggestionType::AskServer)
        ));
        assert!(matches!(
            PermissionTargetParser.usage().0,
            steel_protocol::packets::game::ArgumentType::GameProfile
        ));
    }

    #[test]
    fn permission_key_parser_suggests_catalog_entries() {
        let mut catalog = PermissionCatalog::new();
        catalog.insert(
            PermissionKey::parse("steel.command.steelperms.user.allow")
                .expect("permission key parses"),
            PermissionCatalogSource::Command,
        );
        catalog.insert(
            PermissionKey::parse("minecraft.command.gamemode.creative")
                .expect("permission key parses"),
            PermissionCatalogSource::Command,
        );
        let context = CatalogContext {
            catalog,
            context_catalog: PermissionContextCatalog::new(),
        };

        let suggestions =
            PermissionKeyParser.suggest("steel.command", &ParsedArguments::default(), &context);
        let texts = suggestions
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();

        assert_eq!(texts, vec!["steel.command.steelperms.user.allow"]);
    }

    #[test]
    fn permission_rule_expression_parser_accepts_context_selectors() {
        let mut reader =
            CommandReader::new("minecraft.command.gamemode{world=lobby:spawn,plugin:region=spawn}");
        let value = PermissionRuleExpressionParser
            .parse(&mut reader, &TestContext)
            .expect("permission expression parses");

        assert!(
            matches!(value, ParsedArgument::PermissionRuleExpression(expression)
                if expression.key().as_str() == "minecraft.command.gamemode"
                    && expression.to_string()
                        == "minecraft.command.gamemode{world=lobby:spawn,plugin:region=spawn}")
        );
    }

    #[test]
    fn permission_rule_expression_parser_reports_expression_errors() {
        let mut reader = CommandReader::new("steel.fly{plugin:region=spawn=bad}");
        let error = PermissionRuleExpressionParser
            .parse(&mut reader, &TestContext)
            .expect_err("invalid expression should fail");

        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::InvalidPermissionExpression(value)
                if value.contains("invalid permission context value")
        ));
    }

    #[test]
    fn permission_rule_expression_parser_suggests_catalog_keys_and_contexts() {
        let mut catalog = PermissionCatalog::new();
        catalog.insert(
            PermissionKey::parse("minecraft.command.gamemode").expect("permission key parses"),
            PermissionCatalogSource::Command,
        );
        let mut context_catalog = PermissionContextCatalog::new();
        context_catalog.insert_value(
            PermissionContextKey::parse("plugin:region").expect("context key parses"),
            "spawn",
            PermissionContextCatalogSource::Config,
        );
        let context = CatalogContext {
            catalog,
            context_catalog,
        };

        let key_suggestions = PermissionRuleExpressionParser
            .suggest("minecraft.command", &ParsedArguments::default(), &context)
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();
        let context_key_suggestions = PermissionRuleExpressionParser
            .suggest(
                "minecraft.command.gamemode{p",
                &ParsedArguments::default(),
                &context,
            )
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();
        let context_value_suggestions = PermissionRuleExpressionParser
            .suggest(
                "minecraft.command.gamemode{plugin:region=s",
                &ParsedArguments::default(),
                &context,
            )
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();
        let next_context_key_suggestions = PermissionRuleExpressionParser
            .suggest(
                "minecraft.command.gamemode{domain=lobby,",
                &ParsedArguments::default(),
                &context,
            )
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();

        assert_eq!(key_suggestions, vec!["minecraft.command.gamemode"]);
        assert_eq!(
            context_key_suggestions,
            vec!["minecraft.command.gamemode{plugin:region="]
        );
        assert_eq!(
            context_value_suggestions,
            vec!["minecraft.command.gamemode{plugin:region=spawn}"]
        );
        assert_eq!(
            next_context_key_suggestions,
            vec![
                "minecraft.command.gamemode{domain=lobby,plugin:region=",
                "minecraft.command.gamemode{domain=lobby,world=",
            ]
        );
    }

    #[test]
    fn player_parser_suggests_selectors_without_live_server() {
        let suggestions =
            PlayerParser::multiple().suggest("@", &ParsedArguments::default(), &TestContext);
        let texts = suggestions
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();

        assert_eq!(texts, vec!["@a", "@p", "@r", "@s"]);
    }

    #[test]
    fn single_player_parser_does_not_suggest_multi_player_selector() {
        let suggestions =
            PlayerParser::one().suggest("@", &ParsedArguments::default(), &TestContext);
        let texts = suggestions
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();

        assert_eq!(texts, vec!["@p", "@r", "@s"]);
    }

    #[test]
    fn entity_parser_suggests_selectors_without_live_server() {
        let suggestions =
            EntityParser::multiple().suggest("@", &ParsedArguments::default(), &TestContext);
        let texts = suggestions
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();

        assert_eq!(texts, vec!["@a", "@p", "@r", "@s"]);
    }

    #[test]
    fn entity_summon_parser_resolves_default_namespace() {
        init_test_entities();

        let mut reader = CommandReader::new("pig");
        let value = EntitySummonParser
            .parse(&mut reader, &TestContext)
            .expect("entity type parses");

        assert!(matches!(
            value,
            ParsedArgument::EntityType(entity_type) if entity_type == &vanilla_entities::PIG
        ));
    }

    #[test]
    fn item_parser_resolves_default_namespace() {
        init_test_registry();

        let mut reader = CommandReader::new("stone");
        let value = ItemParser
            .parse(&mut reader, &TestContext)
            .expect("item parses");

        assert!(matches!(
            value,
            ParsedArgument::Item(item) if item == &vanilla_items::ITEMS.stone
        ));
    }

    #[test]
    fn enchantment_parser_resolves_default_namespace() {
        init_test_registry();

        let mut reader = CommandReader::new("sharpness");
        let value = EnchantmentParser
            .parse(&mut reader, &TestContext)
            .expect("enchantment parses");

        assert!(matches!(
            value,
            ParsedArgument::Enchantment(enchantment) if enchantment == &vanilla_enchantments::SHARPNESS
        ));
    }

    #[test]
    fn structure_parser_resolves_default_namespace() {
        init_test_registry();

        let mut reader = CommandReader::new("mineshaft");
        let value = StructureParser
            .parse(&mut reader, &TestContext)
            .expect("structure parses");

        assert!(matches!(
            value,
            ParsedArgument::Structure(structure) if structure.query_name() == "minecraft:mineshaft"
        ));
    }

    #[test]
    fn domain_parser_requires_live_server_context() {
        let mut reader = CommandReader::new("minecraft");
        let error = DomainParser
            .parse(&mut reader, &TestContext)
            .expect_err("domain parser requires a server");

        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::MissingCommandContext("server")
        ));
    }

    #[test]
    fn world_parser_requires_live_server_context() {
        let mut reader = CommandReader::new("minecraft:overworld");
        let error = WorldParser
            .parse(&mut reader, &TestContext)
            .expect_err("world parser requires a server");

        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::MissingCommandContext("server")
        ));
    }

    #[test]
    fn block_pos_parser_requires_position_context() {
        let mut reader = CommandReader::new("1 2 3");
        let error = BlockPosParser
            .parse(&mut reader, &TestContext)
            .expect_err("block position parser requires a position");

        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::MissingCommandContext("position")
        ));
    }

    #[test]
    fn vec3_parser_centers_absolute_xz_without_decimal() {
        let mut reader = CommandReader::new("1 2 3.0");
        let value = Vec3Parser
            .parse(&mut reader, &PositionedContext)
            .expect("vec3 parses");

        assert!(matches!(value, ParsedArgument::Vec3(pos) if pos == DVec3::new(1.5, 2.0, 3.0)));
    }

    #[test]
    fn rotation_parser_accepts_and_normalizes_absolute_rotation() {
        let mut reader = CommandReader::new("181 -181");
        let value = RotationParser
            .parse(&mut reader, &TestContext)
            .expect("rotation parses");

        assert!(matches!(value, ParsedArgument::Rotation((-179.0, 179.0))));
    }

    #[test]
    fn component_parser_consumes_remaining_input() {
        let mut reader = CommandReader::new("{text:\"hello world\"}");
        let value = ComponentParser
            .parse(&mut reader, &TestContext)
            .expect("component parses");

        assert!(matches!(value, ParsedArgument::Component(_)));
        assert_eq!(reader.remaining(), "");
    }

    #[test]
    fn time_parser_accepts_units_and_rounds_to_ticks() {
        let mut reader = CommandReader::new("1.5s");
        let value = TimeParser
            .parse(&mut reader, &TestContext)
            .expect("time parses");

        assert!(matches!(value, ParsedArgument::I32(30)));
    }

    #[test]
    fn time_parser_suggests_units_for_numeric_prefix() {
        let suggestions = TimeParser.suggest("10", &ParsedArguments::default(), &TestContext);
        let texts = suggestions
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();

        assert_eq!(texts, vec!["10d", "10s", "10t"]);
    }
}
