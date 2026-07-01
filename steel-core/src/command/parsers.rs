//! Graph-native command argument parsers.

mod block;
mod function;
mod game;
mod item_predicate;
mod item_stack;
mod loot_predicate;
mod nbt;
mod permission;
mod position;
mod resource;
mod scoreboard;
mod selector;
mod slot;
mod target;
mod text;
mod world;

pub use block::BlockPredicateParser;
pub use function::CommandFunctionParser;
pub use game::GameModeParser;
pub use item_predicate::ItemPredicateParser;
pub use item_stack::ItemStackParser;
pub use loot_predicate::LootPredicateParser;
pub use nbt::NbtPathParser;
pub use permission::{PermissionGroupParser, PermissionKeyParser, PermissionRuleExpressionParser};
pub use position::{BlockPosParser, HeightmapParser, RotationParser, Vec3Parser};
pub(crate) use resource::parse_resource_identifier;
pub use resource::{
    BiomeParser, EnchantmentParser, EntitySummonParser, ItemParser, StructureParser,
};
pub use scoreboard::{DoubleRangeParser, IntRangeParser, ObjectiveParser, ScoreHolderParser};
pub use slot::ItemSlotsParser;
pub use target::{EntityParser, PermissionTargetParser, PlayerParser};
pub use text::{ComponentParser, TimeParser};
pub use world::{DomainParser, WorldParser};

#[cfg(test)]
mod tests {
    use glam::DVec3;
    use steel_protocol::packets::game::{ArgumentType, SuggestionType};
    use steel_registry::{
        REGISTRY, data_components::vanilla_components, item_stack::ItemStack,
        loot_table::RuntimeLootCondition, test_support::init_test_registry, vanilla_attributes,
        vanilla_biomes, vanilla_blocks, vanilla_enchantments, vanilla_entities, vanilla_items,
    };

    use crate::{
        chunk::heightmap::HeightmapType,
        command::{
            graph::{
                CommandArgumentParser, CommandFunctionArgumentValue, CommandParseErrorKind,
                ItemPredicateMatchError, ItemPredicateTarget, ItemPredicateTerm,
                LootPredicateArgumentValue, ParsedArgument, ParsedArguments,
            },
            parsers::{
                BiomeParser, BlockPosParser, BlockPredicateParser, CommandFunctionParser,
                ComponentParser, DomainParser, DoubleRangeParser, EnchantmentParser, EntityParser,
                EntitySummonParser, GameModeParser, HeightmapParser, IntRangeParser, ItemParser,
                ItemPredicateParser, ItemSlotsParser, ItemStackParser, LootPredicateParser,
                NbtPathParser, ObjectiveParser, PermissionKeyParser,
                PermissionRuleExpressionParser, PermissionTargetParser, PlayerParser,
                RotationParser, ScoreHolderParser, StructureParser, TimeParser, Vec3Parser,
                WorldParser,
            },
            reader::CommandReader,
            requirement::{
                CommandInputContext, CommandSourceKind, PermissionExpr, RequirementContext,
            },
        },
        entity::{ENTITIES, init_test_entities},
        permission::{
            PermissionCatalog, PermissionCatalogSource, PermissionContextCatalog,
            PermissionContextCatalogSource, PermissionContextKey, PermissionKey,
        },
    };
    use steel_utils::{Identifier, types::GameType};

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

    struct SelectorPermissionContext;

    impl RequirementContext for SelectorPermissionContext {
        fn source_kind(&self) -> CommandSourceKind {
            CommandSourceKind::Player
        }

        fn has_permission(&self, _permission: &PermissionExpr) -> bool {
            true
        }
    }

    impl CommandInputContext for SelectorPermissionContext {}

    struct SelectorOnlyPermissionContext;

    impl RequirementContext for SelectorOnlyPermissionContext {
        fn source_kind(&self) -> CommandSourceKind {
            CommandSourceKind::Player
        }

        fn has_permission(&self, permission: &PermissionExpr) -> bool {
            matches!(
                permission,
                PermissionExpr::Key(key)
                    if key.as_str() == crate::command::ENTITY_SELECTOR_PERMISSION_KEY
            )
        }
    }

    impl CommandInputContext for SelectorOnlyPermissionContext {}

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
    fn objective_parser_accepts_brigadier_unquoted_string() {
        let mut reader = CommandReader::new("kills next");
        let value = ObjectiveParser
            .parse(&mut reader, &TestContext)
            .expect("objective parses");

        assert!(matches!(value, ParsedArgument::ScoreboardObjective(name) if name == "kills"));
        assert_eq!(reader.remaining(), " next");
    }

    #[test]
    fn scoreboard_parsers_use_native_client_types() {
        let (argument_type, suggestion_type) =
            ObjectiveParser.client_parser().into_protocol_argument();
        assert!(matches!(argument_type, ArgumentType::Objective));
        assert!(matches!(suggestion_type, Some(SuggestionType::AskServer)));

        let (argument_type, suggestion_type) = ScoreHolderParser::multiple()
            .client_parser()
            .into_protocol_argument();
        assert!(matches!(
            argument_type,
            ArgumentType::ScoreHolder { flags: 1 }
        ));
        assert!(matches!(suggestion_type, Some(SuggestionType::AskServer)));

        let (argument_type, suggestion_type) =
            IntRangeParser.client_parser().into_protocol_argument();
        assert!(matches!(argument_type, ArgumentType::IntRange));
        assert!(suggestion_type.is_none());
    }

    #[test]
    fn score_holder_parser_accepts_fake_names_and_wildcard() {
        let mut reader = CommandReader::new("#hidden");
        let value = ScoreHolderParser::one()
            .parse(&mut reader, &TestContext)
            .expect("score holder parses");
        assert!(
            matches!(value, ParsedArgument::ScoreHolders(crate::command::graph::ScoreHolderArgumentValue::Holders(holders)) if holders[0].name() == "#hidden")
        );

        let mut reader = CommandReader::new("*");
        let value = ScoreHolderParser::multiple()
            .parse(&mut reader, &TestContext)
            .expect("wildcard parses");
        assert!(matches!(
            value,
            ParsedArgument::ScoreHolders(crate::command::graph::ScoreHolderArgumentValue::Wildcard)
        ));
    }

    #[test]
    fn score_holder_suggestions_respect_selector_permission() {
        let denied = ScoreHolderParser::multiple()
            .suggest("@", &ParsedArguments::default(), &TestContext)
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();
        assert!(denied.is_empty());

        let allowed = ScoreHolderParser::multiple()
            .suggest("@", &ParsedArguments::default(), &SelectorPermissionContext)
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();
        assert!(allowed.iter().any(|text| text == "@a"));
        assert!(allowed.iter().any(|text| text == "@e"));
        assert!(allowed.iter().any(|text| text == "@n"));

        let single = ScoreHolderParser::one()
            .suggest("@", &ParsedArguments::default(), &SelectorPermissionContext)
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();
        assert!(!single.iter().any(|text| text == "@a"));
        assert!(!single.iter().any(|text| text == "@e"));
        assert!(single.iter().any(|text| text == "@n"));
    }

    #[test]
    fn int_range_parser_accepts_vanilla_range_forms() {
        for (input, matching, missing) in [
            ("5", 5, 4),
            ("5..", 10, 4),
            ("..5", 4, 6),
            ("3..5", 4, 6),
            ("-5..-3", -4, 0),
        ] {
            let mut reader = CommandReader::new(input);
            let ParsedArgument::IntRange(range) = IntRangeParser
                .parse(&mut reader, &TestContext)
                .expect("range parses")
            else {
                panic!("expected int range argument");
            };
            assert!(range.matches(matching));
            assert!(!range.matches(missing));
        }
    }

    #[test]
    fn int_range_parser_rejects_empty_and_swapped_ranges() {
        let mut reader = CommandReader::new("..");
        let error = IntRangeParser
            .parse(&mut reader, &TestContext)
            .expect_err("empty range rejects");
        assert_eq!(
            error.kind(),
            &CommandParseErrorKind::InvalidIntegerRange("..".to_owned())
        );

        let mut reader = CommandReader::new("5..3");
        let error = IntRangeParser
            .parse(&mut reader, &TestContext)
            .expect_err("swapped range rejects");
        assert_eq!(error.kind(), &CommandParseErrorKind::SwappedIntegerRange);
    }

    #[test]
    fn double_range_parser_accepts_vanilla_range_forms() {
        for (input, matching, missing) in [
            ("5", 5.0, 4.0),
            ("5.5..", 10.0, 4.0),
            ("..5.5", 4.0, 6.0),
            ("3.25..5.5", 4.0, 6.0),
            ("-5.5..-3.25", -4.0, 0.0),
        ] {
            let mut reader = CommandReader::new(input);
            let ParsedArgument::DoubleRange(range) = DoubleRangeParser
                .parse(&mut reader, &TestContext)
                .expect("range parses")
            else {
                panic!("expected double range argument");
            };
            assert!(range.matches(matching));
            assert!(!range.matches(missing));
        }
    }

    #[test]
    fn double_range_parser_rejects_empty_swapped_and_non_finite_ranges() {
        let mut reader = CommandReader::new("..");
        let error = DoubleRangeParser
            .parse(&mut reader, &TestContext)
            .expect_err("empty range rejects");
        assert_eq!(
            error.kind(),
            &CommandParseErrorKind::InvalidDoubleRange("..".to_owned())
        );

        let mut reader = CommandReader::new("5.0..3.0");
        let error = DoubleRangeParser
            .parse(&mut reader, &TestContext)
            .expect_err("swapped range rejects");
        assert_eq!(error.kind(), &CommandParseErrorKind::SwappedDoubleRange);

        let mut reader = CommandReader::new("NaN");
        let error = DoubleRangeParser
            .parse(&mut reader, &TestContext)
            .expect_err("non-finite range rejects");
        assert_eq!(
            error.kind(),
            &CommandParseErrorKind::InvalidDoubleRange("NaN".to_owned())
        );
    }

    #[test]
    fn double_range_parser_uses_float_range_client_parser() {
        let (argument_type, suggestion_type) =
            DoubleRangeParser.client_parser().into_protocol_argument();
        assert!(matches!(argument_type, ArgumentType::FloatRange));
        assert!(suggestion_type.is_none());
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
    fn player_parser_hides_selector_suggestions_without_permission() {
        let suggestions =
            PlayerParser::multiple().suggest("@", &ParsedArguments::default(), &TestContext);
        let texts = suggestions
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();

        assert!(texts.is_empty());
    }

    #[test]
    fn player_parser_rejects_selector_without_permission() {
        let mut reader = CommandReader::new("@a");
        let error = match PlayerParser::multiple().parse(&mut reader, &TestContext) {
            Ok(_) => panic!("selector unexpectedly parsed"),
            Err(error) => error,
        };

        assert_eq!(
            error.kind(),
            &CommandParseErrorKind::EntitySelectorsNotAllowed
        );
    }

    #[test]
    fn permission_target_parser_rejects_selector_without_permission() {
        let mut reader = CommandReader::new("@a");
        let error = match PermissionTargetParser.parse(&mut reader, &TestContext) {
            Ok(_) => panic!("selector unexpectedly parsed"),
            Err(error) => error,
        };

        assert_eq!(
            error.kind(),
            &CommandParseErrorKind::EntitySelectorsNotAllowed
        );
    }

    #[test]
    fn player_parser_suggests_selectors_without_live_server() {
        let suggestions = PlayerParser::multiple().suggest(
            "@",
            &ParsedArguments::default(),
            &SelectorPermissionContext,
        );
        let texts = suggestions
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();

        assert_eq!(texts, vec!["@a", "@p", "@r", "@s"]);
    }

    #[test]
    fn single_player_parser_does_not_suggest_multi_player_selector() {
        let suggestions = PlayerParser::one().suggest(
            "@",
            &ParsedArguments::default(),
            &SelectorPermissionContext,
        );
        let texts = suggestions
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();

        assert_eq!(texts, vec!["@p", "@r", "@s"]);
    }

    #[test]
    fn entity_parser_suggests_selectors_without_live_server() {
        let suggestions = EntityParser::multiple().suggest(
            "@",
            &ParsedArguments::default(),
            &SelectorPermissionContext,
        );
        let texts = suggestions
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();

        assert_eq!(texts, vec!["@a", "@e", "@p", "@r", "@s", "@n"]);
    }

    #[test]
    fn entity_parser_suggests_selector_option_keys() {
        let suggestions = EntityParser::multiple().suggest(
            "@e[",
            &ParsedArguments::default(),
            &SelectorPermissionContext,
        );
        let texts = suggestions
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();

        assert!(texts.iter().any(|text| text == "@e[type="));
        assert!(texts.iter().any(|text| text == "@e[sort="));
        assert!(texts.iter().any(|text| text == "@e[advancements="));
        assert!(texts.iter().any(|text| text == "@e[predicate="));
    }

    #[test]
    fn entity_parser_treats_advancements_option_as_set_once_for_suggestions() {
        let suggestions = EntityParser::multiple().suggest(
            "@e[advancements={},",
            &ParsedArguments::default(),
            &SelectorPermissionContext,
        );
        let texts = suggestions
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();

        assert!(
            !texts
                .iter()
                .any(|text| text == "@e[advancements={},advancements=")
        );
        assert!(
            texts
                .iter()
                .any(|text| text == "@e[advancements={},predicate=")
        );
    }

    #[test]
    fn entity_parser_hides_selector_options_without_advanced_permission() {
        let root_suggestions = EntityParser::multiple()
            .suggest(
                "@",
                &ParsedArguments::default(),
                &SelectorOnlyPermissionContext,
            )
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();
        let option_suggestions = EntityParser::multiple()
            .suggest(
                "@e[",
                &ParsedArguments::default(),
                &SelectorOnlyPermissionContext,
            )
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();

        assert!(root_suggestions.iter().any(|text| text == "@e"));
        assert!(option_suggestions.is_empty());
    }

    #[test]
    fn entity_parser_suggests_selector_option_values() {
        init_test_registry();

        let sort_suggestions = EntityParser::multiple()
            .suggest(
                "@e[sort=f",
                &ParsedArguments::default(),
                &SelectorPermissionContext,
            )
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();
        let gamemode_suggestions = EntityParser::multiple()
            .suggest(
                "@e[gamemode=!s",
                &ParsedArguments::default(),
                &SelectorPermissionContext,
            )
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();
        let type_suggestions = EntityParser::multiple()
            .suggest(
                "@e[type=pig",
                &ParsedArguments::default(),
                &SelectorPermissionContext,
            )
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();

        assert_eq!(sort_suggestions, vec!["@e[sort=furthest"]);
        assert!(
            gamemode_suggestions
                .iter()
                .any(|text| text == "@e[gamemode=!survival")
        );
        assert!(
            type_suggestions
                .iter()
                .any(|text| text == "@e[type=minecraft:pig")
        );
    }

    #[test]
    fn entity_parser_suggests_selector_entity_type_tags() {
        init_test_registry();

        let tag_suggestions = EntityParser::multiple()
            .suggest(
                "@e[type=#s",
                &ParsedArguments::default(),
                &SelectorPermissionContext,
            )
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();
        let inverted_tag_suggestions = EntityParser::multiple()
            .suggest(
                "@e[type=!#r",
                &ParsedArguments::default(),
                &SelectorPermissionContext,
            )
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();

        assert!(
            tag_suggestions
                .iter()
                .any(|text| text == "@e[type=#minecraft:skeletons")
        );
        assert!(
            inverted_tag_suggestions
                .iter()
                .any(|text| text == "@e[type=!#minecraft:raiders")
        );
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
    fn entity_summon_parser_accepts_summonable_types_without_factories() {
        init_test_entities();
        assert!(vanilla_entities::VILLAGER.summonable);
        assert!(
            !ENTITIES
                .get()
                .expect("entity registry is initialized")
                .has_factory(&vanilla_entities::VILLAGER)
        );

        let mut reader = CommandReader::new("villager");
        let value = EntitySummonParser
            .parse(&mut reader, &TestContext)
            .expect("summonable entity type parses even without a Steel factory");

        assert!(matches!(
            value,
            ParsedArgument::EntityType(entity_type) if entity_type == &vanilla_entities::VILLAGER
        ));
    }

    #[test]
    fn entity_summon_parser_suggests_summonable_types_without_factories() {
        init_test_entities();
        assert!(vanilla_entities::VILLAGER.summonable);
        assert!(
            !ENTITIES
                .get()
                .expect("entity registry is initialized")
                .has_factory(&vanilla_entities::VILLAGER)
        );

        let suggestions = EntitySummonParser
            .suggest("vill", &ParsedArguments::default(), &TestContext)
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();

        assert!(suggestions.iter().any(|text| text == "minecraft:villager"));
    }

    #[test]
    fn resource_identifier_helper_uses_vanilla_default_namespace() {
        assert_eq!(
            super::parse_resource_identifier("stone"),
            Some(Identifier::vanilla_static("stone"))
        );
        assert_eq!(
            super::parse_resource_identifier(":stone"),
            Some(Identifier::vanilla_static("stone"))
        );
        assert_eq!(
            super::parse_resource_identifier("steel:data"),
            Some(Identifier::new_static("steel", "data"))
        );
        assert!(super::parse_resource_identifier("Steel:data").is_none());
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
    fn item_parser_suggestions_match_after_resource_splitters() {
        init_test_registry();

        let suggestions = ItemParser.suggest("planks", &ParsedArguments::default(), &TestContext);

        assert!(
            suggestions
                .iter()
                .any(|suggestion| suggestion.text == "minecraft:oak_planks")
        );
    }

    #[test]
    fn item_stack_parser_accepts_component_patch_syntax() {
        init_test_registry();

        let mut reader = CommandReader::new("stone[max_stack_size=1,!damage] next");
        let ParsedArgument::ItemStack(stack) = ItemStackParser
            .parse(&mut reader, &TestContext)
            .expect("item stack parses")
        else {
            panic!("expected item stack");
        };

        assert!(stack.is(&vanilla_items::ITEMS.stone));
        assert_eq!(
            stack.get(vanilla_components::MAX_STACK_SIZE).copied(),
            Some(1)
        );
        assert!(stack.patch().is_removed(&vanilla_components::DAMAGE.key));
        assert_eq!(reader.remaining(), " next");
    }

    #[test]
    fn item_stack_parser_rejects_repeated_components() {
        init_test_registry();

        let mut reader = CommandReader::new("stone[damage=1,damage=2]");
        let error = ItemStackParser
            .parse(&mut reader, &TestContext)
            .expect_err("repeated components are rejected");

        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::InvalidItemStack(value)
                if value == "repeated item component 'minecraft:damage'"
        ));
    }

    #[test]
    fn item_stack_parser_rejects_placeholder_component_values() {
        init_test_registry();

        let mut reader = CommandReader::new("stone[custom_data={foo:1}]");
        let error = ItemStackParser
            .parse(&mut reader, &TestContext)
            .expect_err("placeholder components are rejected for set values");

        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::InvalidItemStack(value)
                if value == "unsupported item component 'minecraft:custom_data'"
        ));
    }

    #[test]
    fn item_stack_parser_uses_native_client_type() {
        let (argument_type, suggestion_type) =
            ItemStackParser.client_parser().into_protocol_argument();
        assert!(matches!(argument_type, ArgumentType::ItemStack));
        assert!(matches!(suggestion_type, Some(SuggestionType::AskServer)));
    }

    #[test]
    fn item_stack_parser_suggests_components_with_assignment_suffix() {
        init_test_registry();

        let suggestions =
            ItemStackParser.suggest("stone[da", &ParsedArguments::default(), &TestContext);
        assert!(
            suggestions
                .iter()
                .any(|suggestion| suggestion.text == "stone[minecraft:damage=")
        );

        let removal_suggestions =
            ItemStackParser.suggest("stone[!da", &ParsedArguments::default(), &TestContext);
        assert!(
            removal_suggestions
                .iter()
                .any(|suggestion| suggestion.text == "stone[!minecraft:damage")
        );
    }

    #[test]
    fn item_slots_parser_matches_vanilla_slot_ranges() {
        for (input, expected_slots) in [
            ("contents", vec![0]),
            ("container.5", vec![5]),
            ("container.*", (0..54).collect()),
            ("hotbar.*", (0..9).collect()),
            ("inventory.0", vec![9]),
            ("enderchest.26", vec![226]),
            ("mob.inventory.*", (300..308).collect()),
            ("horse.14", vec![514]),
            ("weapon", vec![98]),
            ("weapon.offhand", vec![99]),
            ("weapon.*", vec![98, 99]),
            ("armor.head", vec![103]),
            ("armor.*", vec![103, 102, 101, 100, 105]),
            ("saddle", vec![106]),
            ("horse.chest", vec![499]),
            ("player.cursor", vec![499]),
            ("player.crafting.3", vec![503]),
        ] {
            let mut reader = CommandReader::new(input);
            let ParsedArgument::ItemSlots(range) = ItemSlotsParser
                .parse(&mut reader, &TestContext)
                .expect("item slots parse")
            else {
                panic!("expected item slots");
            };

            assert_eq!(range.name(), input);
            assert_eq!(range.slots(), expected_slots.as_slice());
        }
    }

    #[test]
    fn item_slots_parser_rejects_unknown_names() {
        let mut reader = CommandReader::new("container.54");
        let error = ItemSlotsParser
            .parse(&mut reader, &TestContext)
            .expect_err("unknown slot range is rejected");

        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::InvalidItemSlot(value) if value == "container.54"
        ));
    }

    #[test]
    fn item_slots_parser_uses_native_client_type() {
        let (argument_type, suggestion_type) =
            ItemSlotsParser.client_parser().into_protocol_argument();
        assert!(matches!(argument_type, ArgumentType::ItemSlots));
        assert!(suggestion_type.is_none());
    }

    #[test]
    fn item_slots_parser_suggests_vanilla_slot_names() {
        let suggestions =
            ItemSlotsParser.suggest("weapon.", &ParsedArguments::default(), &TestContext);
        let texts = suggestions
            .into_iter()
            .map(|suggestion| suggestion.text)
            .collect::<Vec<_>>();

        assert!(texts.contains(&"weapon.mainhand".to_owned()));
        assert!(texts.contains(&"weapon.offhand".to_owned()));
        assert!(texts.contains(&"weapon.*".to_owned()));
    }

    #[test]
    fn loot_predicate_parser_accepts_registry_references() {
        let mut reader = CommandReader::new("custom:test run");
        let ParsedArgument::LootPredicate(predicate) = LootPredicateParser
            .parse(&mut reader, &TestContext)
            .expect("loot predicate parses")
        else {
            panic!("expected loot predicate");
        };

        assert!(matches!(
            predicate,
            LootPredicateArgumentValue::Reference(ref id) if id == &Identifier::new("custom", "test")
        ));
        assert_eq!(reader.remaining(), " run");

        let mut reader = CommandReader::new("test run");
        let ParsedArgument::LootPredicate(predicate) = LootPredicateParser
            .parse(&mut reader, &TestContext)
            .expect("default namespace predicate parses")
        else {
            panic!("expected loot predicate");
        };

        assert!(matches!(
            predicate,
            LootPredicateArgumentValue::Reference(ref id) if id == &Identifier::vanilla_static("test")
        ));
        assert_eq!(reader.remaining(), " run");
    }

    #[test]
    fn loot_predicate_parser_decodes_inline_conditions() {
        let mut reader = CommandReader::new("{condition:\"minecraft:killed_by_player\"} run");
        let ParsedArgument::LootPredicate(predicate) = LootPredicateParser
            .parse(&mut reader, &TestContext)
            .expect("inline compound predicate parses")
        else {
            panic!("expected loot predicate");
        };

        assert!(matches!(
            predicate,
            LootPredicateArgumentValue::Inline(RuntimeLootCondition::KilledByPlayer)
        ));
        assert_eq!(reader.remaining(), " run");

        let mut reader = CommandReader::new("[{condition:\"minecraft:killed_by_player\"}] run");
        let ParsedArgument::LootPredicate(predicate) = LootPredicateParser
            .parse(&mut reader, &TestContext)
            .expect("inline list predicate parses")
        else {
            panic!("expected loot predicate");
        };

        assert!(matches!(
            predicate,
            LootPredicateArgumentValue::Inline(RuntimeLootCondition::AllOf(_))
        ));
        assert_eq!(reader.remaining(), " run");
    }

    #[test]
    fn loot_predicate_parser_rejects_non_condition_snbt() {
        let mut reader = CommandReader::new("{foo:1} run");
        let error = LootPredicateParser
            .parse(&mut reader, &TestContext)
            .expect_err("compound without condition is not a loot condition");

        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::InvalidLootPredicate(value) if value == "missing field 'condition'"
        ));
    }

    #[test]
    fn loot_predicate_parser_reports_invalid_references() {
        let mut reader = CommandReader::new("custom:test:bad");
        let error = LootPredicateParser
            .parse(&mut reader, &TestContext)
            .expect_err("invalid reference is rejected");

        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::InvalidLootPredicate(value) if value == "custom:test:bad"
        ));
    }

    #[test]
    fn loot_predicate_parser_uses_native_client_type() {
        let (argument_type, suggestion_type) =
            LootPredicateParser.client_parser().into_protocol_argument();

        assert!(matches!(argument_type, ArgumentType::LootPredicate));
        assert!(matches!(suggestion_type, Some(SuggestionType::AskServer)));
    }

    #[test]
    fn command_function_parser_accepts_functions_and_tags() {
        let mut reader = CommandReader::new("foo run");
        let ParsedArgument::CommandFunction(target) = CommandFunctionParser
            .parse(&mut reader, &TestContext)
            .expect("function parses")
        else {
            panic!("expected command function");
        };
        assert!(matches!(
            target,
            CommandFunctionArgumentValue::Function(id)
                if id == Identifier::vanilla_static("foo")
        ));
        assert_eq!(reader.remaining(), " run");

        let mut reader = CommandReader::new("#steel:load next");
        let ParsedArgument::CommandFunction(target) = CommandFunctionParser
            .parse(&mut reader, &TestContext)
            .expect("function tag parses")
        else {
            panic!("expected command function tag");
        };
        assert!(matches!(
            target,
            CommandFunctionArgumentValue::Tag(id)
                if id == Identifier::new_static("steel", "load")
        ));
        assert_eq!(reader.remaining(), " next");
    }

    #[test]
    fn command_function_parser_rejects_invalid_identifiers() {
        let mut reader = CommandReader::new("custom:test:bad");
        let error = CommandFunctionParser
            .parse(&mut reader, &TestContext)
            .expect_err("invalid function rejects");

        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::InvalidCommandFunction(value)
                if value == "custom:test:bad"
        ));
    }

    #[test]
    fn command_function_parser_uses_native_client_type() {
        let (argument_type, suggestion_type) = CommandFunctionParser
            .client_parser()
            .into_protocol_argument();

        assert!(matches!(argument_type, ArgumentType::Function));
        assert!(matches!(suggestion_type, Some(SuggestionType::AskServer)));
    }

    #[test]
    fn item_predicate_parser_accepts_vanilla_target_forms() {
        init_test_registry();

        let mut reader = CommandReader::new("stone run");
        let ParsedArgument::ItemPredicate(predicate) = ItemPredicateParser
            .parse(&mut reader, &TestContext)
            .expect("item predicate parses")
        else {
            panic!("expected item predicate");
        };
        assert!(matches!(
            predicate.target(),
            ItemPredicateTarget::Item(item) if *item == &vanilla_items::ITEMS.stone
        ));
        assert!(predicate.conditions().is_empty());
        assert_eq!(reader.remaining(), " run");

        let mut reader = CommandReader::new("#logs [ count = 1 ] next");
        let ParsedArgument::ItemPredicate(predicate) = ItemPredicateParser
            .parse(&mut reader, &TestContext)
            .expect("tag item predicate parses")
        else {
            panic!("expected item predicate");
        };
        assert!(matches!(
            predicate.target(),
            ItemPredicateTarget::Tag { key, items } if key == &Identifier::vanilla_static("logs") && !items.is_empty()
        ));
        assert_eq!(predicate.conditions().len(), 1);
        assert_eq!(reader.remaining(), " next");

        let mut reader = CommandReader::new("*[]");
        let ParsedArgument::ItemPredicate(predicate) = ItemPredicateParser
            .parse(&mut reader, &TestContext)
            .expect("any item predicate parses")
        else {
            panic!("expected item predicate");
        };
        assert!(matches!(predicate.target(), ItemPredicateTarget::Any));
        assert!(predicate.conditions().is_empty());
    }

    #[test]
    fn item_predicate_parser_preserves_condition_ast() {
        init_test_registry();

        let mut reader = CommandReader::new("stone[count=1,!damage|count~{min:2}]");
        let ParsedArgument::ItemPredicate(predicate) = ItemPredicateParser
            .parse(&mut reader, &TestContext)
            .expect("item predicate parses")
        else {
            panic!("expected item predicate");
        };

        assert_eq!(predicate.conditions().len(), 2);
        let first = &predicate.conditions()[0].alternatives()[0];
        assert!(matches!(
            first,
            ItemPredicateTerm::ComponentValue { key, value }
                if key == &Identifier::vanilla_static("count")
                    && value == &simdnbt::owned::NbtTag::Int(1)
        ));

        let second = predicate.conditions()[1].alternatives();
        assert_eq!(second.len(), 2);
        assert!(matches!(
            &second[0],
            ItemPredicateTerm::Not(term)
                if matches!(
                    term.as_ref(),
                    ItemPredicateTerm::ComponentPresence { key }
                        if key == &Identifier::vanilla_static("damage")
                )
        ));
        assert!(matches!(
            &second[1],
            ItemPredicateTerm::PredicateValue { key, value }
                if key == &Identifier::vanilla_static("count")
                    && matches!(value, simdnbt::owned::NbtTag::Compound(_))
        ));
    }

    #[test]
    fn item_predicate_parser_treats_component_tilde_empty_as_presence() {
        init_test_registry();

        let mut reader = CommandReader::new("stone[custom_name~{}]");
        let ParsedArgument::ItemPredicate(predicate) = ItemPredicateParser
            .parse(&mut reader, &TestContext)
            .expect("item predicate parses")
        else {
            panic!("expected item predicate");
        };

        let condition = &predicate.conditions()[0].alternatives()[0];
        assert!(matches!(
            condition,
            ItemPredicateTerm::ComponentPresence { key }
                if key == &Identifier::vanilla_static("custom_name")
        ));
    }

    #[test]
    fn item_predicate_parser_rejects_non_empty_component_existence_predicates() {
        init_test_registry();

        let mut reader = CommandReader::new("stone[custom_name~{value:1}]");
        let error = ItemPredicateParser
            .parse(&mut reader, &TestContext)
            .expect_err("non-empty component existence predicate rejects");

        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::InvalidItemPredicate(value)
                if value == "item component existence predicate 'minecraft:custom_name' requires an empty compound"
        ));
    }

    #[test]
    fn item_predicate_parser_rejects_transient_exact_components() {
        init_test_registry();

        let mut reader = CommandReader::new("stone[creative_slot_lock]");
        let error = ItemPredicateParser
            .parse(&mut reader, &TestContext)
            .expect_err("transient exact component rejects");

        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::InvalidItemPredicate(value)
                if value == "unknown item component 'minecraft:creative_slot_lock'"
        ));

        let mut reader = CommandReader::new("stone[creative_slot_lock~{}]");
        let ParsedArgument::ItemPredicate(predicate) = ItemPredicateParser
            .parse(&mut reader, &TestContext)
            .expect("transient component existence predicate parses")
        else {
            panic!("expected item predicate");
        };

        let condition = &predicate.conditions()[0].alternatives()[0];
        assert!(matches!(
            condition,
            ItemPredicateTerm::ComponentPresence { key }
                if key == &Identifier::vanilla_static("creative_slot_lock")
        ));
    }

    #[test]
    fn item_predicate_matches_item_tags_and_count_ranges() {
        init_test_registry();

        let predicate = parse_item_predicate_for_test("#logs[count={min:2,max:3}]");
        assert!(
            predicate
                .matches_stack(&ItemStack::with_count(&vanilla_items::ITEMS.oak_log, 3))
                .expect("predicate evaluates")
        );
        assert!(
            !predicate
                .matches_stack(&ItemStack::with_count(&vanilla_items::ITEMS.oak_log, 4))
                .expect("predicate evaluates")
        );
        assert!(
            !predicate
                .matches_stack(&ItemStack::with_count(&vanilla_items::ITEMS.stone, 3))
                .expect("predicate evaluates")
        );
    }

    #[test]
    fn item_predicate_matches_implemented_component_values() {
        init_test_registry();

        let predicate = parse_item_predicate_for_test("stone[max_stack_size=64]");
        assert!(
            predicate
                .matches_stack(&ItemStack::new(&vanilla_items::ITEMS.stone))
                .expect("predicate evaluates")
        );

        let predicate = parse_item_predicate_for_test("stone[max_stack_size=1]");
        assert!(
            !predicate
                .matches_stack(&ItemStack::new(&vanilla_items::ITEMS.stone))
                .expect("predicate evaluates")
        );
    }

    #[test]
    fn item_predicate_matches_damage_component_predicate() {
        init_test_registry();

        let mut sword = ItemStack::new(&vanilla_items::ITEMS.diamond_sword);
        sword.set_damage_value(7);

        let predicate = parse_item_predicate_for_test(
            "diamond_sword[damage~{damage:{min:7,max:7},durability:{min:1}}]",
        );
        assert!(
            predicate
                .matches_stack(&sword)
                .expect("predicate evaluates")
        );

        let predicate = parse_item_predicate_for_test("diamond_sword[damage~{damage:{max:6}}]");
        assert!(
            !predicate
                .matches_stack(&sword)
                .expect("predicate evaluates")
        );

        let predicate = parse_item_predicate_for_test("stone[damage~{}]");
        assert!(
            !predicate
                .matches_stack(&ItemStack::new(&vanilla_items::ITEMS.stone))
                .expect("predicate evaluates")
        );
    }

    #[test]
    fn item_predicate_matches_enchantment_component_predicates() {
        init_test_registry();

        let mut sword = ItemStack::new(&vanilla_items::ITEMS.diamond_sword);
        sword.set_enchantments(&[(Identifier::vanilla_static("sharpness"), 3)], false);

        let predicate = parse_item_predicate_for_test(
            "diamond_sword[enchantments~[{enchantments:\"minecraft:sharpness\",levels:{min:2}}]]",
        );
        assert!(
            predicate
                .matches_stack(&sword)
                .expect("predicate evaluates")
        );

        let predicate = parse_item_predicate_for_test(
            "diamond_sword[enchantments~[{enchantments:\"minecraft:sharpness\",levels:{max:2}}]]",
        );
        assert!(
            !predicate
                .matches_stack(&sword)
                .expect("predicate evaluates")
        );

        let predicate = parse_item_predicate_for_test("diamond_sword[enchantments~[{levels:3}]]");
        assert!(
            predicate
                .matches_stack(&sword)
                .expect("predicate evaluates")
        );
    }

    #[test]
    fn item_predicate_matches_enchantment_tag_predicates() {
        init_test_registry();

        let mut sword = ItemStack::new(&vanilla_items::ITEMS.diamond_sword);
        sword.set_enchantments(&[(Identifier::vanilla_static("fire_aspect"), 1)], false);

        let predicate = parse_item_predicate_for_test(
            "diamond_sword[enchantments~[{enchantments:\"#minecraft:smelts_loot\"}]]",
        );
        assert!(
            predicate
                .matches_stack(&sword)
                .expect("predicate evaluates")
        );
    }

    #[test]
    fn item_predicate_matches_stored_enchantment_component_predicates() {
        init_test_registry();

        let mut enchantments = vanilla_components::ItemEnchantments::empty();
        enchantments.set(Identifier::vanilla_static("sharpness"), 2);
        let mut book = ItemStack::new(&vanilla_items::ITEMS.enchanted_book);
        book.set(vanilla_components::STORED_ENCHANTMENTS, enchantments);

        let predicate = parse_item_predicate_for_test(
            "enchanted_book[stored_enchantments~[{enchantments:\"minecraft:sharpness\",levels:2}]]",
        );
        assert!(predicate.matches_stack(&book).expect("predicate evaluates"));

        let predicate = parse_item_predicate_for_test(
            "enchanted_book[stored_enchantments~[{enchantments:\"minecraft:sharpness\",levels:3}]]",
        );
        assert!(!predicate.matches_stack(&book).expect("predicate evaluates"));
    }

    #[test]
    fn item_predicate_matches_attribute_modifier_component_predicates() {
        init_test_registry();

        let mut sword = ItemStack::new(&vanilla_items::ITEMS.diamond_sword);
        sword.set(
            vanilla_components::ATTRIBUTE_MODIFIERS,
            test_attribute_modifiers(),
        );

        let predicate = parse_item_predicate_for_test(
            "diamond_sword[attribute_modifiers~{modifiers:{contains:[{attribute:\"minecraft:attack_damage\",id:\"minecraft:test_modifier\",amount:{min:3.5,max:4.5},operation:\"add_value\",slot:\"mainhand\"}],size:1}}]",
        );
        assert!(
            predicate
                .matches_stack(&sword)
                .expect("predicate evaluates")
        );

        let predicate = parse_item_predicate_for_test(
            "diamond_sword[attribute_modifiers~{modifiers:{contains:[{attribute:[\"minecraft:attack_speed\",\"minecraft:attack_damage\"],amount:4.0}]}}]",
        );
        assert!(
            predicate
                .matches_stack(&sword)
                .expect("predicate evaluates")
        );

        let predicate = parse_item_predicate_for_test(
            "diamond_sword[attribute_modifiers~{modifiers:{contains:[{slot:\"offhand\"}]}}]",
        );
        assert!(
            !predicate
                .matches_stack(&sword)
                .expect("predicate evaluates")
        );
    }

    #[test]
    fn item_predicate_matches_attribute_modifier_count_predicates() {
        init_test_registry();

        let mut sword = ItemStack::new(&vanilla_items::ITEMS.diamond_sword);
        sword.set(
            vanilla_components::ATTRIBUTE_MODIFIERS,
            test_attribute_modifiers(),
        );

        let predicate = parse_item_predicate_for_test(
            "diamond_sword[attribute_modifiers~{modifiers:{count:[{test:{operation:\"add_value\"},count:1}]}}]",
        );
        assert!(
            predicate
                .matches_stack(&sword)
                .expect("predicate evaluates")
        );

        let predicate = parse_item_predicate_for_test(
            "diamond_sword[attribute_modifiers~{modifiers:{count:[{test:{attribute:\"minecraft:attack_speed\"},count:1}]}}]",
        );
        assert!(
            !predicate
                .matches_stack(&sword)
                .expect("predicate evaluates")
        );
    }

    #[test]
    fn item_predicate_reports_malformed_component_predicates() {
        init_test_registry();

        let predicate = parse_item_predicate_for_test("diamond_sword[damage~5]");
        let error = predicate
            .matches_stack(&ItemStack::new(&vanilla_items::ITEMS.diamond_sword))
            .expect_err("malformed predicate reports an error");
        assert!(matches!(
            error,
            ItemPredicateMatchError::MalformedComponentPredicate(key)
                if key == Identifier::vanilla_static("damage")
        ));

        let predicate = parse_item_predicate_for_test(
            "diamond_sword[enchantments~{enchantments:\"minecraft:sharpness\"}]",
        );
        let error = predicate
            .matches_stack(&ItemStack::new(&vanilla_items::ITEMS.diamond_sword))
            .expect_err("malformed predicate reports an error");
        assert!(matches!(
            error,
            ItemPredicateMatchError::MalformedComponentPredicate(key)
                if key == Identifier::vanilla_static("enchantments")
        ));

        let predicate = parse_item_predicate_for_test(
            "diamond_sword[enchantments~[{enchantments:\"minecraft:missing\"}]]",
        );
        let error = predicate
            .matches_stack(&ItemStack::new(&vanilla_items::ITEMS.diamond_sword))
            .expect_err("malformed predicate reports an error");
        assert!(matches!(
            error,
            ItemPredicateMatchError::MalformedComponentPredicate(key)
                if key == Identifier::vanilla_static("enchantments")
        ));

        let predicate = parse_item_predicate_for_test("diamond_sword[attribute_modifiers~5]");
        let error = predicate
            .matches_stack(&ItemStack::new(&vanilla_items::ITEMS.diamond_sword))
            .expect_err("malformed predicate reports an error");
        assert!(matches!(
            error,
            ItemPredicateMatchError::MalformedComponentPredicate(key)
                if key == Identifier::vanilla_static("attribute_modifiers")
        ));

        let predicate = parse_item_predicate_for_test(
            "diamond_sword[attribute_modifiers~{modifiers:{contains:{attribute:\"minecraft:attack_damage\"}}}]",
        );
        let error = predicate
            .matches_stack(&ItemStack::new(&vanilla_items::ITEMS.diamond_sword))
            .expect_err("malformed predicate reports an error");
        assert!(matches!(
            error,
            ItemPredicateMatchError::MalformedComponentPredicate(key)
                if key == Identifier::vanilla_static("attribute_modifiers")
        ));
    }

    #[test]
    fn item_predicate_reports_unsupported_component_predicates() {
        init_test_registry();

        let predicate =
            parse_item_predicate_for_test("potion[potion_contents~{potion:\"minecraft:water\"}]");
        let error = predicate
            .matches_stack(&ItemStack::new(&vanilla_items::ITEMS.potion))
            .expect_err("unsupported predicate reports an error");
        assert!(matches!(
            error,
            ItemPredicateMatchError::UnsupportedComponentPredicate(key)
                if key == Identifier::vanilla_static("potion_contents")
        ));
    }

    #[test]
    fn item_predicate_parser_rejects_unknown_registry_keys() {
        init_test_registry();

        let mut reader = CommandReader::new("missing_item");
        let error = ItemPredicateParser
            .parse(&mut reader, &TestContext)
            .expect_err("unknown item rejects");
        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::InvalidItemPredicate(value)
                if value == "unknown item 'minecraft:missing_item'"
        ));

        let mut reader = CommandReader::new("stone[missing_component=1]");
        let error = ItemPredicateParser
            .parse(&mut reader, &TestContext)
            .expect_err("unknown component rejects");
        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::InvalidItemPredicate(value)
                if value == "unknown item component 'minecraft:missing_component'"
        ));
    }

    #[test]
    fn item_predicate_parser_uses_native_client_type() {
        let (argument_type, suggestion_type) =
            ItemPredicateParser.client_parser().into_protocol_argument();
        assert!(matches!(argument_type, ArgumentType::ItemPredicate));
        assert!(matches!(suggestion_type, Some(SuggestionType::AskServer)));
    }

    #[test]
    fn item_predicate_parser_suggests_targets_and_components() {
        init_test_registry();

        let item_suggestions =
            ItemPredicateParser.suggest("planks", &ParsedArguments::default(), &TestContext);
        assert!(
            item_suggestions
                .iter()
                .any(|suggestion| suggestion.text == "minecraft:oak_planks")
        );

        let tag_suggestions =
            ItemPredicateParser.suggest("#log", &ParsedArguments::default(), &TestContext);
        assert!(
            tag_suggestions
                .iter()
                .any(|suggestion| suggestion.text == "#minecraft:logs")
        );

        let component_suggestions =
            ItemPredicateParser.suggest("stone[da", &ParsedArguments::default(), &TestContext);
        assert!(
            component_suggestions
                .iter()
                .any(|suggestion| suggestion.text == "stone[minecraft:damage")
        );

        let count_suggestions =
            ItemPredicateParser.suggest("stone[co", &ParsedArguments::default(), &TestContext);
        assert!(
            count_suggestions
                .iter()
                .any(|suggestion| suggestion.text == "stone[minecraft:count")
        );

        let transient_suggestions = ItemPredicateParser.suggest(
            "stone[creative",
            &ParsedArguments::default(),
            &TestContext,
        );
        assert!(
            !transient_suggestions
                .iter()
                .any(|suggestion| suggestion.text == "stone[minecraft:creative_slot_lock")
        );
    }

    fn parse_item_predicate_for_test(
        input: &str,
    ) -> crate::command::graph::ItemPredicateArgumentValue {
        let mut reader = CommandReader::new(input);
        let ParsedArgument::ItemPredicate(predicate) = ItemPredicateParser
            .parse(&mut reader, &TestContext)
            .expect("item predicate parses")
        else {
            panic!("expected item predicate");
        };
        predicate
    }

    fn test_attribute_modifiers() -> vanilla_components::ItemAttributeModifiers {
        vanilla_components::ItemAttributeModifiers {
            modifiers: vec![vanilla_components::ItemAttributeModifierEntry {
                attribute: vanilla_attributes::ATTACK_DAMAGE,
                id: Identifier::vanilla_static("test_modifier"),
                amount: 4.0,
                operation: vanilla_components::AttributeModifierOperation::AddValue,
                slot: vanilla_components::EquipmentSlotGroup::MainHand,
                display: vanilla_components::ItemAttributeModifierDisplay::Default,
            }],
        }
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
    fn biome_parser_resolves_default_namespace() {
        init_test_registry();

        let mut reader = CommandReader::new("plains");
        let value = BiomeParser
            .parse(&mut reader, &TestContext)
            .expect("biome parses");

        assert!(matches!(
            value,
            ParsedArgument::Biome(biome) if biome.matches_biome(&vanilla_biomes::PLAINS)
        ));
    }

    #[test]
    fn biome_parser_resolves_tags() {
        init_test_registry();

        let mut reader = CommandReader::new("#is_overworld");
        let value = BiomeParser
            .parse(&mut reader, &TestContext)
            .expect("biome tag parses");

        assert!(matches!(
            value,
            ParsedArgument::Biome(biome) if biome.matches_biome(&vanilla_biomes::PLAINS)
        ));
    }

    #[test]
    fn block_predicate_parser_resolves_properties_and_nbt() {
        init_test_registry();

        let mut reader = CommandReader::new("oak_log[axis=y]{id:'minecraft:barrel'} tail");
        let value = BlockPredicateParser
            .parse(&mut reader, &TestContext)
            .expect("block predicate parses");

        let ParsedArgument::BlockPredicate(predicate) = value else {
            panic!("expected block predicate");
        };
        let y_state = REGISTRY
            .blocks
            .state_id_from_block_defaulted_properties(&vanilla_blocks::OAK_LOG, [("axis", "y")])
            .expect("oak log y-axis state exists");
        let x_state = REGISTRY
            .blocks
            .state_id_from_block_defaulted_properties(&vanilla_blocks::OAK_LOG, [("axis", "x")])
            .expect("oak log x-axis state exists");

        assert!(predicate.matches_state(y_state));
        assert!(!predicate.matches_state(x_state));
        assert_eq!(
            predicate
                .nbt()
                .and_then(|nbt| nbt.string("id"))
                .map(|id| id.to_str().into_owned()),
            Some("minecraft:barrel".to_owned())
        );
        assert_eq!(reader.remaining(), " tail");
    }

    #[test]
    fn block_predicate_parser_resolves_tags() {
        init_test_registry();

        let mut reader = CommandReader::new("#logs[axis=y]");
        let value = BlockPredicateParser
            .parse(&mut reader, &TestContext)
            .expect("block tag predicate parses");

        let ParsedArgument::BlockPredicate(predicate) = value else {
            panic!("expected block predicate");
        };
        let y_log = REGISTRY
            .blocks
            .state_id_from_block_defaulted_properties(&vanilla_blocks::OAK_LOG, [("axis", "y")])
            .expect("oak log y-axis state exists");
        let stone = REGISTRY.blocks.get_default_state_id(&vanilla_blocks::STONE);

        assert!(predicate.matches_state(y_log));
        assert!(!predicate.matches_state(stone));
    }

    #[test]
    fn block_predicate_parser_suggestions_match_default_namespace() {
        init_test_registry();

        let block_suggestions =
            BlockPredicateParser.suggest("oak_l", &ParsedArguments::default(), &TestContext);
        let tag_suggestions =
            BlockPredicateParser.suggest("#lo", &ParsedArguments::default(), &TestContext);

        assert!(
            block_suggestions
                .iter()
                .any(|suggestion| suggestion.text == "minecraft:oak_log")
        );
        assert!(
            tag_suggestions
                .iter()
                .any(|suggestion| suggestion.text == "#minecraft:logs")
        );
    }

    #[test]
    fn nbt_path_parser_reads_one_command_argument() {
        let mut reader = CommandReader::new("Items[{id:\"minecraft:stone\"}].Count run seed");
        let value = NbtPathParser
            .parse(&mut reader, &TestContext)
            .expect("nbt path parses");

        let ParsedArgument::NbtPath(path) = value else {
            panic!("expected nbt path");
        };
        assert_eq!(path.as_str(), "Items[{id:\"minecraft:stone\"}].Count");
        assert_eq!(reader.remaining(), " run seed");
    }

    #[test]
    fn nbt_path_parser_uses_native_client_type() {
        let (argument, suggestion) = NbtPathParser.client_parser().into_protocol_argument();

        assert!(matches!(
            argument,
            steel_protocol::packets::game::ArgumentType::NbtPath
        ));
        assert!(suggestion.is_none());
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
    fn block_pos_parser_reports_incomplete_position() {
        let mut reader = CommandReader::new("1 2");
        let error = BlockPosParser
            .parse(&mut reader, &TestContext)
            .expect_err("block position is incomplete");

        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::InvalidBlockPos(value) if value == "1 2"
        ));
        assert_eq!(error.cursor(), 0);
    }

    #[test]
    fn heightmap_parser_accepts_vanilla_names_case_insensitively() {
        let mut reader = CommandReader::new("MOTION_BLOCKING_NO_LEAVES");
        let value = HeightmapParser
            .parse(&mut reader, &TestContext)
            .expect("heightmap parses");

        assert!(matches!(
            value,
            ParsedArgument::Heightmap(HeightmapType::MotionBlockingNoLeaves)
        ));
    }

    #[test]
    fn heightmap_parser_suggests_kept_after_worldgen_types() {
        let suggestions =
            HeightmapParser.suggest("motion", &ParsedArguments::default(), &TestContext);
        let texts = suggestions
            .iter()
            .map(|suggestion| suggestion.text.as_str())
            .collect::<Vec<_>>();

        assert_eq!(texts, vec!["motion_blocking", "motion_blocking_no_leaves"]);
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
    fn vec3_parser_reports_incomplete_position() {
        let mut reader = CommandReader::new("1 ");
        let error = Vec3Parser
            .parse(&mut reader, &TestContext)
            .expect_err("vec3 is incomplete");

        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::InvalidVec3(value) if value == "1"
        ));
        assert_eq!(error.cursor(), 0);
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
    fn rotation_parser_reports_incomplete_rotation() {
        let mut reader = CommandReader::new("90");
        let error = RotationParser
            .parse(&mut reader, &TestContext)
            .expect_err("rotation is incomplete");

        assert!(matches!(
            error.kind(),
            CommandParseErrorKind::InvalidRotation(value) if value == "90"
        ));
        assert_eq!(error.cursor(), 0);
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
