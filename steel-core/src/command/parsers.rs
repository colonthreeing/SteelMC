//! Graph-native command argument parsers.

use std::sync::Arc;

use rand::seq::IteratorRandom;
use steel_protocol::packets::game::{ArgumentType, SuggestionEntry, SuggestionType};
use steel_utils::{
    Identifier,
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
        graph::{
            CommandArgumentParser, CommandParseError, CommandParseErrorKind, ParsedArgument,
            ParsedArguments,
        },
        reader::{CommandReader, StringMode},
        requirement::CommandInputContext,
    },
    entity::{Entity, LivingEntity},
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

        Ok(ParsedArgument::Component(component))
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
    use crate::command::{
        graph::{CommandArgumentParser, CommandParseErrorKind, ParsedArgument, ParsedArguments},
        parsers::{
            ComponentParser, DomainParser, EntityParser, GameModeParser, PlayerParser, TimeParser,
            WorldParser,
        },
        reader::CommandReader,
        requirement::{CommandInputContext, CommandSourceKind, PermissionExpr, RequirementContext},
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
