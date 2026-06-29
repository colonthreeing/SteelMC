use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::command::commands;
use crate::command::parsers::{ComponentParser, GameModeParser, PermissionKeyParser};
use crate::command::{
    graph::{
        AnchorParser, BoolParser, CommandArgumentClientParser, CommandArgumentParser, CommandGraph,
        CommandGraphError, CommandNodeBuilder, CommandNodeNameError, CommandParseError,
        CommandParseErrorKind, CommandRedirectTarget, CommandResult, FloatParser, IntegerParser,
        LongParser, ParsedArgument, ParsedCommandAction, StringParser, SuggestionResult, argument,
        literal,
    },
    reader::{CommandReader, StringMode},
    requirement::{
        CommandInputContext, CommandSourceKind, PermissionExpr, PermissionKey, Requirement,
        RequirementContext,
    },
};
use crate::permission::{
    PermissionCatalog, PermissionCatalogSource, PermissionEntry, PermissionSet,
};
use steel_protocol::packets::game::{
    ArgumentStringTypeBehavior, ArgumentType, CommandNode as ProtocolCommandNode,
};
use steel_utils::{serial::WriteTo, types::GameType};

struct TestContext {
    source_kind: CommandSourceKind,
    permissions: PermissionSet,
}

struct EmptySuccessParser;

impl CommandArgumentParser for EmptySuccessParser {
    fn parse(
        &self,
        _reader: &mut CommandReader<'_>,
        _context: &dyn CommandInputContext,
    ) -> Result<ParsedArgument, CommandParseError> {
        Ok(ParsedArgument::String(String::new()))
    }

    fn client_parser(&self) -> CommandArgumentClientParser {
        CommandArgumentClientParser::new(
            ArgumentType::String {
                behavior: ArgumentStringTypeBehavior::SingleWord,
            },
            None,
        )
    }

    fn parsed_type(&self) -> &'static str {
        "empty_success"
    }
}

impl RequirementContext for TestContext {
    fn source_kind(&self) -> CommandSourceKind {
        self.source_kind
    }

    fn has_permission(&self, permission: &PermissionExpr) -> bool {
        self.permissions.allows(permission)
    }
}

impl CommandInputContext for TestContext {}

fn player_context() -> TestContext {
    TestContext {
        source_kind: CommandSourceKind::Player,
        permissions: PermissionSet::default(),
    }
}

fn player_context_with(permission: PermissionKey) -> TestContext {
    player_context_with_all([permission])
}

fn player_context_with_all<const N: usize>(permissions: [PermissionKey; N]) -> TestContext {
    player_context_with_entries(permissions.map(PermissionEntry::allow))
}

fn player_context_with_entries<const N: usize>(entries: [PermissionEntry; N]) -> TestContext {
    TestContext {
        source_kind: CommandSourceKind::Player,
        permissions: PermissionSet::from_entries(entries),
    }
}

fn suggestion_texts(result: &SuggestionResult) -> Vec<String> {
    result
        .suggestions
        .iter()
        .map(|suggestion| suggestion.text.clone())
        .collect()
}

fn graph_with_root(root: CommandNodeBuilder) -> CommandGraph {
    CommandGraph::new()
        .with_root(root)
        .expect("test command root registers")
}

fn graph_registration_error(root: CommandNodeBuilder) -> CommandGraphError {
    match CommandGraph::new().with_root(root) {
        Ok(_) => panic!("test command root should be rejected"),
        Err(error) => error,
    }
}

#[test]
fn registration_rejects_invalid_node_names() {
    let error = graph_registration_error(literal("bad name"));
    assert_eq!(
        error,
        CommandGraphError::InvalidLiteralName {
            name: "bad name".to_owned(),
            source: CommandNodeNameError::ContainsWhitespace
        }
    );

    let error = graph_registration_error(literal("root").then(argument("", BoolParser)));
    assert_eq!(
        error,
        CommandGraphError::InvalidArgumentName {
            name: String::new(),
            source: CommandNodeNameError::Empty
        }
    );
}

#[test]
fn registration_rejects_terminal_argument_children() {
    let error = graph_registration_error(
        literal("root").then(
            argument("message", StringParser::new(StringMode::GreedyPhrase))
                .then(literal("tail").executes(|_, _| Ok(CommandResult::success()))),
        ),
    );

    assert_eq!(
        error,
        CommandGraphError::TerminalArgumentMustBeLeaf {
            name: "message".to_owned(),
        }
    );
}

#[test]
fn registration_rejects_terminal_argument_redirects() {
    let error = graph_registration_error(
        literal("root").then(
            argument("message", StringParser::new(StringMode::GreedyPhrase))
                .redirects(CommandRedirectTarget::All, |_, _| {
                    Ok(CommandResult::success())
                }),
        ),
    );

    assert_eq!(
        error,
        CommandGraphError::TerminalArgumentMustBeLeaf {
            name: "message".to_owned(),
        }
    );
}

#[test]
fn registration_rejects_server_terminal_argument_children() {
    let error = graph_registration_error(
        literal("tellraw").then(
            argument("message", ComponentParser)
                .then(literal("tail").executes(|_, _| Ok(CommandResult::success()))),
        ),
    );

    assert_eq!(
        error,
        CommandGraphError::TerminalArgumentMustBeLeaf {
            name: "message".to_owned(),
        }
    );
}

#[test]
fn registration_rejects_client_terminal_permission_argument_children() {
    let error = graph_registration_error(
        literal("perm").then(
            argument("permission", PermissionKeyParser)
                .then(literal("tail").executes(|_, _| Ok(CommandResult::success()))),
        ),
    );

    assert_eq!(
        error,
        CommandGraphError::TerminalArgumentMustBeLeaf {
            name: "permission".to_owned(),
        }
    );
}

#[test]
fn registration_rejects_unmergeable_literal_collisions() {
    let graph = graph_with_root(literal("root").executes(|_, _| Ok(CommandResult::success())));
    let Err(error) = graph.with_root(literal("root").executes(|_, _| Ok(CommandResult::success())))
    else {
        panic!("duplicate executable root should be rejected");
    };

    assert_eq!(
        error,
        CommandGraphError::LiteralCollision {
            name: "root".to_owned()
        }
    );
}

#[test]
fn derived_subcommand_permission_requires_literal_node() {
    let root_permission =
        PermissionKey::parse("minecraft.command.root").expect("permission key parses");
    let root =
        literal("root").then(argument("target", BoolParser).requires_subcommand_permission());
    let mut catalog = PermissionCatalog::new();
    let Err(error) = root.resolve_subcommand_permissions(&root_permission, &mut catalog) else {
        panic!("argument node permission marker should be rejected");
    };

    assert_eq!(
        error,
        CommandGraphError::DerivedPermissionRequiresLiteral {
            name: "target".to_owned()
        }
    );
}

#[test]
fn dynamic_argument_permission_requires_registration_resolution() {
    let error = graph_registration_error(literal("root").then(
        argument("gamemode", GameModeParser).requires_argument_permission::<GameType>("gamemode"),
    ));

    assert_eq!(
        error,
        CommandGraphError::UnresolvedDynamicPermission {
            name: "gamemode".to_owned()
        }
    );
}

#[test]
fn derived_subcommand_permission_requires_registration_resolution() {
    let error = graph_registration_error(
        literal("root").then(literal("child").requires_subcommand_permission()),
    );

    assert_eq!(
        error,
        CommandGraphError::UnresolvedDerivedPermission {
            name: "child".to_owned()
        }
    );
}

#[test]
fn dynamic_argument_permission_requires_available_argument() {
    let root_permission =
        PermissionKey::parse("minecraft.command.root").expect("permission key parses");
    let mut catalog = PermissionCatalog::new();
    let Err(error) = literal("root")
        .then(argument("gamemode", GameModeParser).requires_argument_permission::<GameType>("mode"))
        .resolve_subcommand_permissions(&root_permission, &mut catalog)
    else {
        panic!("missing dynamic permission argument should reject registration");
    };

    assert_eq!(
        error,
        CommandGraphError::MissingDynamicPermissionArgument {
            node: "gamemode".to_owned(),
            argument: "mode".to_owned()
        }
    );
}

#[test]
fn dynamic_argument_permission_validates_argument_type() {
    let root_permission =
        PermissionKey::parse("minecraft.command.root").expect("permission key parses");
    let mut catalog = PermissionCatalog::new();
    let Err(error) = literal("root")
        .then(argument("enabled", BoolParser).requires_argument_permission::<GameType>("enabled"))
        .resolve_subcommand_permissions(&root_permission, &mut catalog)
    else {
        panic!("wrong dynamic permission argument type should reject registration");
    };

    assert_eq!(
        error,
        CommandGraphError::WrongDynamicPermissionArgumentType {
            node: "enabled".to_owned(),
            argument: "enabled".to_owned(),
            expected: "gamemode",
            actual: "bool"
        }
    );
}

#[test]
fn permission_resolution_registers_catalog_entries() {
    let root_permission =
        PermissionKey::parse("minecraft.command.gamemode").expect("permission key parses");
    let mut catalog = PermissionCatalog::new();
    commands::gamemode::command()
        .resolve_subcommand_permissions(&root_permission, &mut catalog)
        .expect("gamemode permissions resolve");

    let suggestions = catalog.suggestions("minecraft.command.gamemode");
    assert_eq!(
        suggestions,
        vec![
            "minecraft.command.gamemode.adventure".to_owned(),
            "minecraft.command.gamemode.creative".to_owned(),
            "minecraft.command.gamemode.spectator".to_owned(),
            "minecraft.command.gamemode.survival".to_owned(),
        ]
    );
    assert!(
        catalog
            .entries()
            .all(|entry| entry.sources().contains(&PermissionCatalogSource::Command))
    );
}

#[test]
fn explicit_permission_requirements_register_catalog_entries() {
    let root_permission =
        PermissionKey::parse("minecraft.command.root").expect("permission key parses");
    let admin = PermissionKey::parse("steel.admin").expect("permission key parses");
    let audit = PermissionKey::parse("steel.audit").expect("permission key parses");
    let mut catalog = PermissionCatalog::new();

    literal("root")
        .then(
            literal("admin")
                .requires_permission_expr(PermissionExpr::key(admin) | PermissionExpr::key(audit)),
        )
        .resolve_subcommand_permissions(&root_permission, &mut catalog)
        .expect("permissions resolve");

    assert_eq!(
        catalog.suggestions("steel."),
        vec!["steel.admin".to_owned(), "steel.audit".to_owned()]
    );
}

#[test]
fn gamemode_allows_root_or_dynamic_value_permission() {
    let root_permission =
        PermissionKey::parse("minecraft.command.gamemode").expect("permission key parses");
    let mut catalog = PermissionCatalog::new();
    let root = commands::gamemode::command()
        .resolve_subcommand_permissions(&root_permission, &mut catalog)
        .expect("gamemode permissions resolve");
    let graph = graph_with_root(root);

    let root_only = player_context_with(root_permission.clone());
    assert!(graph.parse("gamemode creative", &root_only).is_ok());

    let creative = player_context_with(
        PermissionKey::parse("minecraft.command.gamemode.creative").expect("permission key parses"),
    );
    assert!(graph.parse("gamemode creative", &creative).is_ok());
    assert!(graph.parse("gamemode survival", &creative).is_err());

    let root_allow_creative_deny = player_context_with_entries([
        PermissionEntry::allow(root_permission),
        PermissionEntry::deny(
            PermissionKey::parse("minecraft.command.gamemode.creative")
                .expect("permission key parses"),
        ),
    ]);
    assert!(
        graph
            .parse("gamemode creative", &root_allow_creative_deny)
            .is_err()
    );
    assert!(
        graph
            .parse("gamemode survival", &root_allow_creative_deny)
            .is_ok()
    );
}

#[test]
fn dynamic_argument_permission_filters_value_suggestions() {
    let root_permission =
        PermissionKey::parse("minecraft.command.gamemode").expect("permission key parses");
    let mut catalog = PermissionCatalog::new();
    let root = commands::gamemode::command()
        .resolve_subcommand_permissions(&root_permission, &mut catalog)
        .expect("gamemode permissions resolve");
    let graph = graph_with_root(root);

    assert!(graph.suggest("gamemode ", &player_context()).is_none());

    let survival = player_context_with(
        PermissionKey::parse("minecraft.command.gamemode.survival").expect("permission key parses"),
    );
    let result = graph
        .suggest("gamemode ", &survival)
        .expect("survival suggestion");

    assert_eq!(suggestion_texts(&result), vec!["survival".to_owned()]);

    let root = player_context_with(root_permission.clone());
    let result = graph.suggest("gamemode ", &root).expect("root suggestions");

    assert_eq!(
        suggestion_texts(&result),
        vec![
            "survival".to_owned(),
            "creative".to_owned(),
            "adventure".to_owned(),
            "spectator".to_owned()
        ]
    );

    let root_without_creative = player_context_with_entries([
        PermissionEntry::allow(root_permission.clone()),
        PermissionEntry::deny(
            PermissionKey::parse("minecraft.command.gamemode.creative")
                .expect("permission key parses"),
        ),
    ]);
    let result = graph
        .suggest("gamemode ", &root_without_creative)
        .expect("root suggestions");

    assert_eq!(
        suggestion_texts(&result),
        vec![
            "survival".to_owned(),
            "adventure".to_owned(),
            "spectator".to_owned()
        ]
    );
}

#[test]
fn denied_dynamic_argument_hides_deeper_suggestions() {
    let root_permission =
        PermissionKey::parse("minecraft.command.gamemode").expect("permission key parses");
    let mut catalog = PermissionCatalog::new();
    let root = commands::gamemode::command()
        .resolve_subcommand_permissions(&root_permission, &mut catalog)
        .expect("gamemode permissions resolve");
    let graph = graph_with_root(root);

    assert!(
        graph
            .suggest("gamemode creative @", &player_context())
            .is_none()
    );

    let creative = player_context_with(
        PermissionKey::parse("minecraft.command.gamemode.creative").expect("permission key parses"),
    );
    let result = graph
        .suggest("gamemode creative @", &creative)
        .expect("target suggestions");

    assert_eq!(
        suggestion_texts(&result),
        vec![
            "@a".to_owned(),
            "@p".to_owned(),
            "@r".to_owned(),
            "@s".to_owned()
        ]
    );
}

#[test]
fn parses_literal_and_integer_argument() {
    let graph = graph_with_root(
        literal("give").then(
            argument("count", IntegerParser::bounded(Some(1), Some(64)))
                .executes(|_, _| Ok(CommandResult::success())),
        ),
    );

    let result = graph
        .parse("/give 12", &player_context())
        .expect("command parses");

    assert_eq!(result.path(), ["give", "count"]);
    assert_eq!(result.arguments().get::<i32>("count"), Ok(12));
}

#[test]
fn argument_parser_must_consume_input() {
    let graph =
        graph_with_root(literal("root").then(
            argument("value", EmptySuccessParser).executes(|_, _| Ok(CommandResult::success())),
        ));

    let error = graph
        .parse("root value", &player_context())
        .expect_err("argument parser that does not consume input should reject parsing");

    assert_eq!(
        error.kind(),
        &CommandParseErrorKind::ArgumentParserDidNotConsumeInput {
            argument: "value".to_owned(),
            parsed_type: "empty_success",
        }
    );
    assert_eq!(error.cursor(), 5);
}

#[test]
fn literals_do_not_partially_match_before_punctuation() {
    let graph = graph_with_root(literal("root").executes(|_, _| Ok(CommandResult::success())));
    let error = graph
        .parse("/root:tail", &player_context())
        .expect_err("literal should not partially match");

    assert_eq!(
        error.kind(),
        &CommandParseErrorKind::ExpectedLiteral("root".to_owned())
    );
    assert_eq!(error.cursor(), 1);
}

#[test]
fn parses_long_argument() {
    let graph =
        graph_with_root(literal("meta").then(
            argument("value", LongParser::new()).executes(|_, _| Ok(CommandResult::success())),
        ));

    let result = graph
        .parse("/meta 9223372036854775807", &player_context())
        .expect("command parses");

    assert_eq!(result.path(), ["meta", "value"]);
    assert_eq!(
        result.arguments().get::<i64>("value"),
        Ok(9_223_372_036_854_775_807)
    );
}

#[test]
fn leading_whitespace_is_not_skipped_at_root() {
    let graph = graph_with_root(literal("list").executes(|_, _| Ok(CommandResult::success())));

    let error = graph
        .parse(" list", &player_context())
        .expect_err("leading whitespace should not be accepted");

    assert_eq!(
        error.kind(),
        &CommandParseErrorKind::ExpectedLiteral("list".to_owned())
    );
    assert_eq!(error.cursor(), 0);
}

#[test]
fn parses_named_arguments_for_executors() {
    let graph = graph_with_root(
        literal("flag")
            .then(argument("enabled", BoolParser).executes(|_, _| Ok(CommandResult::success()))),
    );

    let result = graph
        .parse("flag true", &player_context())
        .expect("command parses");

    assert_eq!(result.arguments().get::<bool>("enabled"), Ok(true));
}

#[test]
fn parses_bounded_float_argument() {
    let graph = graph_with_root(
        literal("speed").then(
            argument("value", FloatParser::bounded(Some(0.0), Some(30.0)))
                .executes(|_, _| Ok(CommandResult::success())),
        ),
    );

    let result = graph
        .parse("speed 1.5", &player_context())
        .expect("command parses");

    assert_eq!(result.arguments().get::<f32>("value"), Ok(1.5));

    let error = graph
        .parse("speed 31.0", &player_context())
        .expect_err("out-of-range float should fail");
    assert_eq!(
        error.kind(),
        &CommandParseErrorKind::FloatTooHigh {
            value: 31.0,
            max: 30.0
        }
    );
}

#[test]
fn quoted_string_argument_keeps_spaces() {
    let graph = graph_with_root(
        literal("say").then(
            argument("message", StringParser::new(StringMode::QuotablePhrase))
                .executes(|_, _| Ok(CommandResult::success())),
        ),
    );

    let result = graph
        .parse("say \"hello world\"", &player_context())
        .expect("command parses");

    assert_eq!(
        result.arguments().get::<String>("message"),
        Ok("hello world".to_owned())
    );
}

#[test]
fn redirect_node_captures_remaining_command_tail() {
    let graph = graph_with_root(literal("execute").then(
        literal("run").redirects(CommandRedirectTarget::All, |_, _| {
            Ok(CommandResult::success())
        }),
    ));

    let result = graph
        .parse("execute run say hello", &player_context())
        .expect("redirect parses");

    assert_eq!(result.path(), ["execute", "run"]);
    let ParsedCommandAction::Redirect(redirect) = &result.action else {
        panic!("expected redirect action");
    };
    assert_eq!(redirect.target, CommandRedirectTarget::All);
    assert_eq!(redirect.command, "say hello");
    assert_eq!(redirect.current_root, "execute");
}

#[test]
fn trailing_data_is_distinct_from_incomplete_command() {
    let graph = graph_with_root(literal("list").executes(|_, _| Ok(CommandResult::success())));

    let error = graph
        .parse("list extra", &player_context())
        .expect_err("extra input should fail");

    assert_eq!(error.kind(), &CommandParseErrorKind::TrailingData);
    assert_eq!(error.cursor(), 5);
}

#[test]
fn branch_errors_prefer_farthest_cursor() {
    let graph = graph_with_root(
        literal("root")
            .then(literal("foo").executes(|_, _| Ok(CommandResult::success())))
            .then(
                literal("bar").then(
                    argument("count", IntegerParser::new())
                        .executes(|_, _| Ok(CommandResult::success())),
                ),
            ),
    );

    let error = graph
        .parse("root bar nope", &player_context())
        .expect_err("invalid integer should fail");

    assert_eq!(
        error.kind(),
        &CommandParseErrorKind::InvalidInteger("nope".to_owned())
    );
    assert_eq!(error.cursor(), 9);
}

#[test]
fn requirements_hide_unusable_nodes() {
    let denied = Arc::new(AtomicBool::new(false));
    let denied_in_executor = Arc::clone(&denied);
    let graph = graph_with_root(
        literal("admin")
            .requires(Requirement::Permission(PermissionExpr::key(
                PermissionKey::parse("steel.admin").expect("key parses"),
            )))
            .executes(move |_, _| {
                denied_in_executor.store(true, Ordering::Relaxed);
                Ok(CommandResult::success())
            }),
    );

    let error = graph
        .parse("admin", &player_context())
        .expect_err("node should be hidden");

    assert_eq!(error.kind(), &CommandParseErrorKind::UnknownCommand);
    assert!(!denied.load(Ordering::Relaxed));
}

#[test]
fn usage_hides_unusable_roots() {
    let graph = CommandGraph::new()
        .with_root(literal("open"))
        .expect("open root registers")
        .with_root(
            literal("admin").requires(Requirement::Permission(PermissionExpr::key(
                PermissionKey::parse("steel.admin").expect("key parses"),
            ))),
        )
        .expect("admin root registers");
    let mut nodes = vec![ProtocolCommandNode::new_root()];
    let mut root_children = Vec::new();

    graph.usage(&mut nodes, &mut root_children, &player_context());

    assert_eq!(
        literal_names(&nodes, &root_children),
        vec!["open".to_owned()]
    );

    let allowed_context =
        player_context_with(PermissionKey::parse("steel.admin").expect("key parses"));
    let mut nodes = vec![ProtocolCommandNode::new_root()];
    let mut root_children = Vec::new();

    graph.usage(&mut nodes, &mut root_children, &allowed_context);

    assert_eq!(
        literal_names(&nodes, &root_children),
        vec!["open".to_owned(), "admin".to_owned()]
    );
}

#[test]
fn usage_marks_permission_nodes_restricted() {
    const RESTRICTED_FLAG: u8 = 32;

    let graph = graph_with_root(literal("admin").requires(Requirement::Permission(
        PermissionExpr::key(PermissionKey::parse("steel.admin").expect("key parses")),
    )));
    let allowed_context =
        player_context_with(PermissionKey::parse("steel.admin").expect("key parses"));
    let mut nodes = vec![ProtocolCommandNode::new_root()];
    let mut root_children = Vec::new();

    graph.usage(&mut nodes, &mut root_children, &allowed_context);

    let admin = usize::try_from(root_children[0]).expect("node index is non-negative");
    assert_ne!(node_flag_byte(&nodes[admin]) & RESTRICTED_FLAG, 0);
}

#[test]
fn root_suggestions_include_partial_literals() {
    let graph = graph_with_root(literal("list"));

    let result = graph
        .suggest("li", &player_context())
        .expect("root suggestion");

    assert_eq!(suggestion_texts(&result), vec!["list".to_owned()]);
    assert_eq!(result.start, 0);
    assert_eq!(result.length, 2);
}

#[test]
fn slash_root_suggestions_keep_client_offset() {
    let graph = graph_with_root(literal("list"));

    let result = graph
        .suggest("/li", &player_context())
        .expect("root suggestion");

    assert_eq!(suggestion_texts(&result), vec!["list".to_owned()]);
    assert_eq!(result.start, 1);
    assert_eq!(result.length, 2);
}

#[test]
fn leading_whitespace_does_not_suggest_roots() {
    let graph = graph_with_root(literal("list"));

    assert!(graph.suggest(" l", &player_context()).is_none());
}

#[test]
fn child_literal_suggestions_use_child_range() {
    let graph = graph_with_root(literal("list").then(literal("uuids")));

    let result = graph
        .suggest("list u", &player_context())
        .expect("child suggestion");

    assert_eq!(suggestion_texts(&result), vec!["uuids".to_owned()]);
    assert_eq!(result.start, 5);
    assert_eq!(result.length, 1);
}

#[test]
fn trailing_space_suggests_children() {
    let graph = graph_with_root(literal("list").then(literal("uuids")));

    let result = graph
        .suggest("list ", &player_context())
        .expect("child suggestion");

    assert_eq!(suggestion_texts(&result), vec!["uuids".to_owned()]);
    assert_eq!(result.start, 5);
    assert_eq!(result.length, 0);
}

#[test]
fn then_all_adds_children_in_order() {
    let graph = graph_with_root(literal("root").then_all([
        literal("one"),
        literal("two"),
        literal("three"),
    ]));

    let result = graph
        .suggest("root ", &player_context())
        .expect("child suggestions");

    assert_eq!(
        suggestion_texts(&result),
        vec!["one".to_owned(), "two".to_owned(), "three".to_owned()]
    );
}

#[test]
fn trailing_space_after_leaf_has_no_stale_suggestion() {
    let graph = graph_with_root(literal("list").then(literal("uuids")));

    assert!(graph.suggest("list uuids ", &player_context()).is_none());
}

#[test]
fn bool_parser_suggests_values() {
    let graph = graph_with_root(literal("flag").then(argument("enabled", BoolParser)));

    let result = graph
        .suggest("flag f", &player_context())
        .expect("bool suggestion");

    assert_eq!(suggestion_texts(&result), vec!["false".to_owned()]);
    assert_eq!(result.start, 5);
    assert_eq!(result.length, 1);
}

fn literal_names(nodes: &[ProtocolCommandNode], indexes: &[i32]) -> Vec<String> {
    indexes
        .iter()
        .filter_map(|index| {
            let index = usize::try_from(*index).ok()?;
            match &nodes[index] {
                ProtocolCommandNode::Literal { name, .. } => Some(name.to_string()),
                ProtocolCommandNode::Root { .. } | ProtocolCommandNode::Argument { .. } => None,
            }
        })
        .collect()
}

fn node_flag_byte(node: &ProtocolCommandNode) -> u8 {
    let mut bytes = Vec::new();
    node.write(&mut bytes).expect("command node writes");
    bytes[0]
}

#[test]
fn duplicate_literal_roots_merge_child_branches() {
    let graph = CommandGraph::new()
        .with_root(literal("root").then(literal("one")))
        .expect("first root branch registers")
        .with_root(literal("root").then(literal("two")))
        .expect("second root branch registers");

    let root_result = graph
        .suggest("ro", &player_context())
        .expect("root suggestion");
    assert_eq!(suggestion_texts(&root_result), vec!["root".to_owned()]);

    let child_result = graph
        .suggest("root ", &player_context())
        .expect("child suggestions");
    assert_eq!(
        suggestion_texts(&child_result),
        vec!["one".to_owned(), "two".to_owned()]
    );
}

#[test]
fn redirect_to_all_suggests_dispatcher_roots() {
    let graph = CommandGraph::new()
        .with_root(literal("give"))
        .expect("give root registers")
        .with_root(literal("execute").then(
            literal("run").redirects(CommandRedirectTarget::All, |_, _| {
                Ok(CommandResult::success())
            }),
        ))
        .expect("execute root registers");

    let result = graph
        .suggest("execute run gi", &player_context())
        .expect("redirect target suggestion");

    assert_eq!(suggestion_texts(&result), vec!["give".to_owned()]);
    assert_eq!(result.start, 12);
    assert_eq!(result.length, 2);
}

#[test]
fn redirect_to_current_suggests_current_root_children() {
    let graph = graph_with_root(
        literal("execute")
            .then(
                literal("anchored").then(
                    argument("anchor", AnchorParser)
                        .redirects(CommandRedirectTarget::Current, |_, _| {
                            Ok(CommandResult::success())
                        }),
                ),
            )
            .then(
                literal("run").redirects(CommandRedirectTarget::All, |_, _| {
                    Ok(CommandResult::success())
                }),
            ),
    );

    let result = graph
        .suggest("execute anchored eyes ru", &player_context())
        .expect("current redirect suggestion");

    assert_eq!(suggestion_texts(&result), vec!["run".to_owned()]);
    assert_eq!(result.start, 22);
    assert_eq!(result.length, 2);
}
