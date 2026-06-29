//! Graph-native command argument parsers.

mod game;
mod permission;
mod position;
mod resource;
mod target;
mod text;
mod world;

pub use game::GameModeParser;
pub use permission::{PermissionGroupParser, PermissionKeyParser, PermissionRuleExpressionParser};
pub use position::{BlockPosParser, RotationParser, Vec3Parser};
pub use resource::{EnchantmentParser, EntitySummonParser, ItemParser, StructureParser};
pub use target::{EntityParser, PermissionTargetParser, PlayerParser};
pub use text::{ComponentParser, TimeParser};
pub use world::{DomainParser, WorldParser};

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
    fn permission_key_parser_reads_one_server_token() {
        let mut reader = CommandReader::new("steel.command.steelperms.* trailing");
        let value = PermissionKeyParser
            .parse(&mut reader, &TestContext)
            .expect("permission key parses");

        assert!(
            matches!(value, ParsedArgument::PermissionKey(permission) if permission.as_str() == "steel.command.steelperms.*")
        );
        assert_eq!(reader.remaining(), " trailing");
    }

    #[test]
    fn permission_management_parsers_request_server_suggestions() {
        let (permission_key_argument, permission_key_suggestion) =
            PermissionKeyParser.client_parser().into_protocol_argument();
        assert!(matches!(
            permission_key_argument,
            steel_protocol::packets::game::ArgumentType::String {
                behavior: steel_protocol::packets::game::ArgumentStringTypeBehavior::GreedyPhrase
            }
        ));
        assert!(matches!(
            permission_key_suggestion,
            Some(SuggestionType::AskServer),
        ));

        let (expression_argument, expression_suggestion) = PermissionRuleExpressionParser
            .client_parser()
            .into_protocol_argument();
        assert!(matches!(
            expression_argument,
            steel_protocol::packets::game::ArgumentType::String {
                behavior: steel_protocol::packets::game::ArgumentStringTypeBehavior::GreedyPhrase
            }
        ));
        assert!(matches!(
            expression_suggestion,
            Some(SuggestionType::AskServer),
        ));

        let (_, group_suggestion) = super::PermissionGroupParser
            .client_parser()
            .into_protocol_argument();
        assert!(matches!(group_suggestion, Some(SuggestionType::AskServer),));

        let (target_argument, _) = PermissionTargetParser
            .client_parser()
            .into_protocol_argument();
        assert!(matches!(
            target_argument,
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
        assert_eq!(error.cursor(), 0);
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
