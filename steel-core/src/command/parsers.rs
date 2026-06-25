//! Graph-native command argument parsers.

use std::{f32::consts::PI, sync::Arc};

use glam::DVec3;
use rand::seq::IteratorRandom;
use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use steel_registry::{REGISTRY, RegistryExt, TaggedRegistryExt, entity_type::EntityTypeRef};
use steel_utils::{
    BlockPos, Identifier,
    translations::{
        ARGUMENT_ENTITY_SELECTOR_ALL_ENTITIES, ARGUMENT_ENTITY_SELECTOR_ALL_PLAYERS,
        ARGUMENT_ENTITY_SELECTOR_NEAREST_ENTITY, ARGUMENT_ENTITY_SELECTOR_NEAREST_PLAYER,
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
            ParsedArguments, StructureArgumentValue,
        },
        reader::{CommandReader, StringMode},
        requirement::CommandInputContext,
    },
    entity::{ENTITIES, Entity, LivingEntity},
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
        if players.is_empty() {
            return Ok(ParsedArgument::Players(Vec::new()));
        }

        let targets = match value.as_str() {
            "@a" => players,
            "@p" => {
                let Some(position) = context.position() else {
                    return Err(CommandParseError::new(
                        CommandParseErrorKind::MissingCommandContext("position"),
                        cursor,
                    ));
                };

                let mut nearest = (f64::MAX, Arc::clone(&players[0]));
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
                let uuid = Uuid::parse_str(name).unwrap_or_else(|_| Uuid::nil());
                let Some(player) = players
                    .into_iter()
                    .find(|player| player.gameprofile.name == name || player.uuid() == uuid)
                else {
                    return Err(CommandParseError::new(
                        CommandParseErrorKind::InvalidPlayer(value),
                        cursor,
                    ));
                };
                vec![player]
            }
        };

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
        if players.is_empty() {
            return Ok(ParsedArgument::Entities(Vec::new()));
        }

        let targets = match value.as_str() {
            "@a" | "@e" => players
                .into_iter()
                .map(|player| player as Arc<dyn LivingEntity + Send + Sync>)
                .collect(),
            "@n" | "@p" => {
                let Some(position) = context.position() else {
                    return Err(CommandParseError::new(
                        CommandParseErrorKind::MissingCommandContext("position"),
                        cursor,
                    ));
                };

                let mut nearest = (f64::MAX, Arc::clone(&players[0]));
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
            name => {
                let uuid = Uuid::parse_str(name).unwrap_or_else(|_| Uuid::nil());
                let Some(player) = players
                    .into_iter()
                    .find(|player| player.gameprofile.name == name || player.uuid() == uuid)
                else {
                    return Err(CommandParseError::new(
                        CommandParseErrorKind::InvalidEntity(value),
                        cursor,
                    ));
                };
                vec![player as Arc<dyn LivingEntity + Send + Sync>]
            }
        };

        Ok(ParsedArgument::Entities(targets))
    }

    fn usage(&self) -> (ArgumentType, Option<SuggestionType>) {
        (
            ArgumentType::Entity {
                flags: u8::from(self.one),
            },
            Some(SuggestionType::AskServer),
        )
    }

    fn suggest(
        &self,
        prefix: &str,
        _arguments: &ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Vec<SuggestionEntry> {
        let mut suggestions = vec![
            SuggestionEntry::with_tooltip("@a", &ARGUMENT_ENTITY_SELECTOR_ALL_PLAYERS),
            SuggestionEntry::with_tooltip("@e", &ARGUMENT_ENTITY_SELECTOR_ALL_ENTITIES),
            SuggestionEntry::with_tooltip("@n", &ARGUMENT_ENTITY_SELECTOR_NEAREST_ENTITY),
            SuggestionEntry::with_tooltip("@p", &ARGUMENT_ENTITY_SELECTOR_NEAREST_PLAYER),
            SuggestionEntry::with_tooltip("@r", &ARGUMENT_ENTITY_SELECTOR_RANDOM_PLAYER),
            SuggestionEntry::with_tooltip("@s", &ARGUMENT_ENTITY_SELECTOR_SELF),
        ];

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
                EntitySummonParser, GameModeParser, ItemParser, PlayerParser, RotationParser,
                StructureParser, TimeParser, Vec3Parser, WorldParser,
            },
            reader::CommandReader,
            requirement::{
                CommandInputContext, CommandSourceKind, PermissionExpr, RequirementContext,
            },
        },
        entity::init_test_entities,
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
    fn entity_parser_suggests_selectors_without_live_server() {
        let suggestions =
            EntityParser::multiple().suggest("@", &ParsedArguments::default(), &TestContext);
        let texts = suggestions
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();

        assert_eq!(texts, vec!["@a", "@e", "@n", "@p", "@r", "@s"]);
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
