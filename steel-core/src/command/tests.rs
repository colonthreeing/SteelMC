use super::{
    CommandCallbackResult, CommandDispatcher, CommandExecutionBudget, CommandRegistration,
    CommandRegistrationError,
};
use crate::command::{
    context::CommandResultCallback,
    error::CommandError,
    functions::CommandFunction,
    graph::{
        CommandGraphError, CommandParseErrorKind, CommandResult, StringParser, argument, literal,
    },
    reader::StringMode,
    requirement::{CommandInputContext, CommandSourceKind, PermissionExpr, RequirementContext},
    sender::CommandSender,
};
use crate::permission::{
    PermissionCatalogSource, PermissionEntry, PermissionKey, PermissionSegment, PermissionSet,
};
use glam::DVec3;
use steel_protocol::packets::game::CommandNode as ProtocolCommandNode;
use steel_registry::test_support::init_test_registry;
use steel_utils::{Identifier, translations};
use text_components::{TextComponent, content::Content};

struct TestContext {
    permissions: PermissionSet,
    position: Option<DVec3>,
}

impl RequirementContext for TestContext {
    fn source_kind(&self) -> CommandSourceKind {
        CommandSourceKind::Player
    }

    fn has_permission(&self, permission: &PermissionExpr) -> bool {
        self.permissions.allows(permission)
    }
}

impl CommandInputContext for TestContext {
    fn position(&self) -> Option<DVec3> {
        self.position
    }
}

fn player_context() -> TestContext {
    TestContext {
        permissions: PermissionSet::default(),
        position: None,
    }
}

fn player_context_with(permission: &str) -> TestContext {
    player_context_with_all([permission])
}

fn player_context_with_all<const N: usize>(permissions: [&str; N]) -> TestContext {
    player_context_with_entries(permissions.map(|permission| {
        PermissionEntry::allow(
            PermissionKey::parse(permission).expect("test permission key parses"),
        )
    }))
}

fn player_context_with_entries<const N: usize>(entries: [PermissionEntry; N]) -> TestContext {
    TestContext {
        permissions: PermissionSet::from_entries(entries),
        position: None,
    }
}

fn positioned_player_context_with(permission: &str) -> TestContext {
    TestContext {
        permissions: PermissionSet::from_entries([allow(permission)]),
        position: Some(DVec3::ZERO),
    }
}

fn allow(permission: &str) -> PermissionEntry {
    PermissionEntry::allow(PermissionKey::parse(permission).expect("test permission key parses"))
}

fn deny(permission: &str) -> PermissionEntry {
    PermissionEntry::deny(PermissionKey::parse(permission).expect("test permission key parses"))
}

fn text_content(component: &TextComponent) -> &str {
    let Content::Text { text } = &component.content else {
        panic!("component should be plain text");
    };
    text
}

fn translation_key(component: &TextComponent) -> &str {
    let Content::Translate(message) = &component.content else {
        panic!("component should be translated");
    };
    &message.key
}

fn command_failed_component(error: CommandError) -> Box<TextComponent> {
    let CommandError::CommandFailed(component) = error else {
        panic!("error should be a command failure");
    };
    component
}

#[test]
fn command_execution_budget_stops_after_limit() {
    let mut budget = CommandExecutionBudget::new(1);

    assert!(budget.consume());
    assert!(!budget.consume());
    assert_eq!(
        budget.stopped,
        Some(super::CommandExecutionStop::SequenceLimit)
    );
}

#[test]
fn command_queue_limit_uses_vanilla_exclusive_boundary() {
    assert!(!super::queue_len_exceeds_max(
        super::MAX_COMMAND_QUEUE_DEPTH
    ));
    assert!(super::queue_len_exceeds_max(
        super::MAX_COMMAND_QUEUE_DEPTH + 1
    ));
}

#[test]
fn command_fork_limit_uses_vanilla_exclusive_boundary() {
    assert!(super::fork_limit_reached(0, 0, 0));
    assert!(!super::fork_limit_reached(0, 1, 2));
    assert!(super::fork_limit_reached(1, 1, 2));
    assert!(super::fork_limit_reached(0, 2, 2));
}

#[test]
fn command_fork_limit_error_uses_vanilla_translation_key() {
    let error = super::command_fork_limit_error(12);
    let CommandError::CommandFailed(component) = error else {
        panic!("fork limit should be a command failure");
    };

    assert_eq!(translation_key(&component), "command.forkLimit");
}

#[test]
fn forked_execution_counts_completed_source_not_command_result() {
    assert_eq!(
        super::execution_success_count(CommandResult::from_return_value(12), false),
        12
    );
    assert_eq!(
        super::execution_success_count(CommandResult::from_return_value(12), true),
        1
    );
    assert_eq!(
        super::execution_success_count(CommandResult::from_return_value(0), true),
        1
    );
}

#[test]
fn command_result_callback_receives_command_return_value() {
    let callback = super::command_callback_result(CommandResult::from_return_value(37));

    assert!(callback.success);
    assert_eq!(callback.result, 37);
}

#[test]
fn function_result_message_uses_vanilla_translation_key() {
    let message = super::function_result_message(&Identifier::new_static("test", "gate"), 37);
    let Content::Translate(message) = &message.content else {
        panic!("function result should be translated");
    };
    let args = message.args.as_ref().expect("function result has args");

    assert_eq!(message.key.as_ref(), "commands.function.result");
    assert_eq!(text_content(&args[0]), "test:gate");
    assert_eq!(text_content(&args[1]), "37");
}

#[test]
fn function_instantiation_error_uses_function_translation_key() {
    let error = super::command_function_instantiation_error(
        super::FunctionInstantiationFailureContext::FunctionCommand,
        &Identifier::new_static("test", "macro"),
        TextComponent::from("missing arguments"),
    );
    let component = command_failed_component(error);
    let Content::Translate(message) = &component.content else {
        panic!("function instantiation error should be translated");
    };
    let args = message.args.as_ref().expect("function error has args");

    assert_eq!(
        message.key.as_ref(),
        "commands.function.instantiationFailure"
    );
    assert_eq!(text_content(&args[0]), "test:macro");
    assert_eq!(text_content(&args[1]), "missing arguments");
}

#[test]
fn function_condition_instantiation_error_uses_execute_translation_key() {
    let error = super::command_function_instantiation_error(
        super::FunctionInstantiationFailureContext::ExecuteCondition,
        &Identifier::new_static("test", "macro"),
        TextComponent::from("missing arguments"),
    );
    let component = command_failed_component(error);
    let Content::Translate(message) = &component.content else {
        panic!("function condition instantiation error should be translated");
    };
    let args = message.args.as_ref().expect("function error has args");

    assert_eq!(
        message.key.as_ref(),
        "commands.execute.function.instantiationFailure"
    );
    assert_eq!(text_content(&args[0]), "test:macro");
    assert_eq!(text_content(&args[1]), "missing arguments");
}

#[test]
fn function_condition_instantiation_keeps_successful_prefix_before_error() {
    let dispatcher = CommandDispatcher::new_empty();
    let first = CommandFunction::new(
        Identifier::new_static("test", "first"),
        ["known".to_owned()],
    );
    let invalid_macro =
        CommandFunction::from_source(Identifier::new_static("test", "macro"), "$echo $(value)")
            .expect("macro function parses");
    let skipped = CommandFunction::new(
        Identifier::new_static("test", "skipped"),
        ["known skipped".to_owned()],
    );

    let context = player_context();
    let prepared =
        dispatcher.instantiate_functions_for_condition(&[first, invalid_macro, skipped], &context);

    assert_eq!(prepared.functions.len(), 1);
    assert_eq!(prepared.functions[0].commands(), ["known"]);
    let failure = prepared.failure.expect("macro instantiation fails");
    assert_eq!(failure.function_id, Identifier::new_static("test", "macro"));
    assert!(
        failure.reason.contains("requires macro arguments"),
        "unexpected failure reason: {}",
        failure.reason
    );
}

#[test]
fn decorated_function_callback_invokes_downstream_callback() {
    let seen = std::sync::Arc::new(steel_utils::locks::SyncMutex::new(Vec::new()));
    let callback_seen = std::sync::Arc::clone(&seen);
    let callback = CommandResultCallback::new(move |result| {
        callback_seen.lock().push(result);
    });
    let result = CommandCallbackResult {
        success: true,
        result: 37,
    };

    let decorated = super::decorated_function_callback(
        CommandSender::SuppressedOutput(std::sync::Arc::new(CommandSender::Console)),
        false,
        Identifier::new_static("test", "gate"),
        callback,
    );
    decorated.on_result(result);

    assert_eq!(*seen.lock(), vec![result]);
}

#[test]
fn function_result_accumulation_matches_vanilla_callback_rule() {
    assert!(!super::should_accumulate_function_results(
        1,
        &CommandResultCallback::new(|_| {})
    ));
    assert!(!super::should_accumulate_function_results(
        2,
        &CommandResultCallback::empty()
    ));
    assert!(super::should_accumulate_function_results(
        2,
        &CommandResultCallback::new(|_| {})
    ));
}

#[test]
fn return_failure_callback_reports_failure() {
    let callback = super::command_callback_result(CommandResult::return_failure());

    assert!(!callback.success);
    assert_eq!(callback.result, 0);
}

#[test]
fn callback_failure_does_not_return_from_command_frame() {
    let result = CommandResult::callback_failure(0);
    let callback = super::command_callback_result(result);

    assert!(!callback.success);
    assert_eq!(callback.result, 0);
    assert!(!result.returns_from_frame());
}

#[test]
fn frame_discard_removes_only_current_frame_prefix() {
    let mut queue = std::collections::VecDeque::from([
        super::QueuedAction::Fallthrough(super::QueuedFrame::for_depth(2)),
        super::QueuedAction::Fallthrough(super::QueuedFrame::for_depth(1)),
        super::QueuedAction::Fallthrough(super::QueuedFrame::for_depth(0)),
        super::QueuedAction::Fallthrough(super::QueuedFrame::for_depth(1)),
    ]);

    super::discard_command_frame(&mut queue, 1);

    assert_eq!(queue.len(), 2);
    assert!(matches!(
        queue.front(),
        Some(super::QueuedAction::Fallthrough(frame)) if frame.depth == 0
    ));
    assert!(matches!(
        queue.back(),
        Some(super::QueuedAction::Fallthrough(frame)) if frame.depth == 1
    ));
}

#[test]
fn queue_next_actions_preserves_order_before_existing_tail() {
    let mut queue = std::collections::VecDeque::from([super::QueuedAction::Fallthrough(
        super::QueuedFrame::for_depth(0),
    )]);

    let mut budget = CommandExecutionBudget::new(1);

    assert!(super::queue_next_actions(
        &mut queue,
        [
            super::QueuedAction::Fallthrough(super::QueuedFrame::for_depth(1)),
            super::QueuedAction::Fallthrough(super::QueuedFrame::for_depth(2)),
        ],
        &mut budget,
    ));

    let depths = queue
        .iter()
        .map(super::QueuedAction::frame_depth)
        .collect::<Vec<_>>();
    assert_eq!(depths, [1, 2, 0]);
}

#[test]
fn redirect_sibling_matching_is_limited_to_same_command_stage() {
    let frame = super::QueuedFrame::for_depth(1);
    let other_frame = super::QueuedFrame::for_depth(2);
    let batch = super::RedirectSiblingBatch {
        active_command: "execute if function test:gate run seed",
        active_fork_stage: 1,
        active_returning: false,
        active_frame: &frame,
        next_command: "execute run seed",
        forked: true,
        returns: false,
    };

    assert!(super::redirect_sibling_matches(
        "execute if function test:gate run seed",
        1,
        false,
        &frame,
        &batch,
    ));
    assert!(!super::redirect_sibling_matches(
        "execute if entity @s run seed",
        1,
        false,
        &frame,
        &batch,
    ));
    assert!(!super::redirect_sibling_matches(
        "execute if function test:gate run seed",
        2,
        false,
        &frame,
        &batch,
    ));
    assert!(!super::redirect_sibling_matches(
        "execute if function test:gate run seed",
        1,
        false,
        &other_frame,
        &batch,
    ));
}

#[test]
fn returning_redirect_without_contexts_queues_fallthrough() {
    let actions = super::redirect_actions(
        "seed".to_owned(),
        Vec::new(),
        false,
        0,
        true,
        super::QueuedFrame::for_depth(2),
    );

    assert_eq!(actions.len(), 1);
    assert!(matches!(
        actions.first(),
        Some(super::QueuedAction::Fallthrough(frame)) if frame.depth == 2
    ));
}

#[test]
fn non_returning_redirect_without_contexts_queues_nothing() {
    let actions = super::redirect_actions(
        "seed".to_owned(),
        Vec::new(),
        false,
        0,
        false,
        super::QueuedFrame::for_depth(2),
    );

    assert!(actions.is_empty());
}

#[test]
fn local_frame_return_invokes_callback_without_dispatch_return() {
    let seen = std::sync::Arc::new(steel_utils::locks::SyncMutex::new(Vec::new()));
    let callback_seen = std::sync::Arc::clone(&seen);
    let frame = super::QueuedFrame::with_return_callback(
        1,
        1,
        crate::command::context::CommandResultCallback::new(move |result| {
            callback_seen.lock().push(result);
        }),
        false,
    );
    let result = CommandCallbackResult {
        success: true,
        result: 7,
    };
    let mut frame_return = None;
    let mut queue = std::collections::VecDeque::from([super::QueuedAction::Fallthrough(
        super::QueuedFrame::for_depth(0),
    )]);

    super::return_from_frame(&mut queue, &frame, result, &mut frame_return);

    assert_eq!(frame_return, None);
    assert_eq!(*seen.lock(), vec![result]);
    assert!(matches!(
        queue.front(),
        Some(super::QueuedAction::Fallthrough(frame)) if frame.depth == 0
    ));
}

#[test]
fn propagating_frame_return_records_dispatch_return_and_discards_prefix() {
    let frame = super::QueuedFrame::with_return_callback(
        2,
        1,
        crate::command::context::CommandResultCallback::empty(),
        true,
    );
    let result = CommandCallbackResult {
        success: true,
        result: 9,
    };
    let mut frame_return = None;
    let mut queue = std::collections::VecDeque::from([
        super::QueuedAction::Fallthrough(super::QueuedFrame::for_depth(1)),
        super::QueuedAction::Fallthrough(super::QueuedFrame::for_depth(0)),
    ]);

    super::return_from_frame(&mut queue, &frame, result, &mut frame_return);

    assert_eq!(frame_return, Some(result));
    assert_eq!(queue.len(), 1);
    assert!(matches!(
        queue.front(),
        Some(super::QueuedAction::Fallthrough(frame)) if frame.depth == 0
    ));
}

#[test]
fn dispatch_result_preserves_non_forked_command_result() {
    assert_eq!(
        super::dispatch_result(12, 37, false),
        CommandResult::from_return_value(37)
    );
}

#[test]
fn dispatch_result_aggregates_completed_forked_contexts() {
    assert_eq!(
        super::dispatch_result(3, 37, true),
        CommandResult::from_success_count(3)
    );
}

#[test]
fn dispatcher_applies_root_command_permissions() {
    init_test_registry();

    let dispatcher = CommandDispatcher::new().expect("built-in commands register");
    let player = player_context();

    assert!(dispatcher.graph.has_root("list", &player));
    assert!(!dispatcher.graph.has_root("give", &player));
    assert!(!dispatcher.graph.has_root("deop", &player));
    assert!(!dispatcher.graph.has_root("function", &player));
    assert!(!dispatcher.graph.has_root("gamemode", &player));
    assert!(!dispatcher.graph.has_root("op", &player));
    assert!(!dispatcher.graph.has_root("return", &player));
    assert!(!dispatcher.graph.has_root("tp", &player));
    assert!(!dispatcher.graph.has_root("teleport", &player));
    assert!(!dispatcher.graph.has_root("steelperms", &player));
    assert!(!dispatcher.graph.has_root("sp", &player));

    let give_player = player_context_with("minecraft.command.give");
    assert!(dispatcher.graph.has_root("give", &give_player));

    let op_player = player_context_with("minecraft.command.op");
    assert!(dispatcher.graph.has_root("op", &op_player));

    let deop_player = player_context_with("minecraft.command.deop");
    assert!(dispatcher.graph.has_root("deop", &deop_player));

    let function_player = player_context_with("minecraft.command.function");
    assert!(dispatcher.graph.has_root("function", &function_player));

    let return_player = player_context_with("minecraft.command.return");
    assert!(dispatcher.graph.has_root("return", &return_player));

    let gamemode_player = player_context_with("minecraft.command.gamemode");
    assert!(dispatcher.graph.has_root("gamemode", &gamemode_player));
    assert!(!dispatcher.graph.has_root("tp", &gamemode_player));

    let gamemode_creative_player = player_context_with("minecraft.command.gamemode.creative");
    assert!(
        dispatcher
            .graph
            .has_root("gamemode", &gamemode_creative_player)
    );

    let teleport_player = player_context_with("minecraft.command.teleport");
    assert!(dispatcher.graph.has_root("tp", &teleport_player));
    assert!(dispatcher.graph.has_root("teleport", &teleport_player));

    let experience_player = player_context_with("minecraft.command.experience");
    assert!(dispatcher.graph.has_root("xp", &experience_player));
    assert!(dispatcher.graph.has_root("experience", &experience_player));

    let steelperms_player = player_context_with("steel.command.steelperms");
    assert!(dispatcher.graph.has_root("steelperms", &steelperms_player));
    assert!(dispatcher.graph.has_root("sp", &steelperms_player));
}

#[test]
fn function_command_parses_direct_macro_arguments() {
    init_test_registry();

    let dispatcher = CommandDispatcher::new().expect("built-in commands register");
    let function_player = player_context_with("minecraft.command.function");
    let parsed = dispatcher
        .graph
        .parse("function test:macro {value:1}", &function_player)
        .expect("function command with direct macro arguments parses");

    assert_eq!(parsed.path(), ["function", "name", "arguments"]);

    let storage = dispatcher
        .graph
        .parse(
            "function test:macro with storage steel:data value",
            &function_player,
        )
        .expect("function command with storage data arguments parses");
    assert_eq!(
        storage.path(),
        ["function", "name", "with", "storage", "source", "path"]
    );

    let block = dispatcher
        .graph
        .parse(
            "function test:macro with block 0 64 0 value",
            &positioned_player_context_with("minecraft.command.function"),
        )
        .expect("function command with block data arguments parses");
    assert_eq!(
        block.path(),
        ["function", "name", "with", "block", "sourcePos", "path"]
    );

    let entity = dispatcher
        .graph
        .parse(
            "function test:macro with entity Steve value",
            &function_player,
        )
        .expect("function command with entity data arguments parses");
    assert_eq!(
        entity.path(),
        ["function", "name", "with", "entity", "source", "path"]
    );
}

#[test]
fn dispatcher_applies_derived_subcommand_permissions() {
    init_test_registry();

    let dispatcher = CommandDispatcher::new().expect("built-in commands register");
    let tick_root_player = player_context_with("minecraft.command.tick");
    assert!(dispatcher.graph.has_root("tick", &tick_root_player));
    assert!(
        dispatcher
            .graph
            .parse("tick freeze", &tick_root_player)
            .is_ok()
    );

    let tick_freeze_player = player_context_with("minecraft.command.tick.freeze");
    assert!(dispatcher.graph.has_root("tick", &tick_freeze_player));
    assert!(
        dispatcher
            .graph
            .parse("tick freeze", &tick_freeze_player)
            .is_ok()
    );

    let tick_root_without_freeze = player_context_with_entries([
        allow("minecraft.command.tick"),
        deny("minecraft.command.tick.freeze"),
    ]);
    assert!(
        dispatcher
            .graph
            .parse("tick freeze", &tick_root_without_freeze)
            .is_err()
    );
}

#[test]
fn execute_run_requires_redirected_command_permission() {
    init_test_registry();

    let dispatcher = CommandDispatcher::new().expect("built-in commands register");
    let execute_only = player_context_with("minecraft.command.execute");

    assert!(
        dispatcher
            .graph
            .parse("execute run gamemode creative", &execute_only)
            .is_err()
    );
}

#[test]
fn execute_run_uses_original_source_permissions_for_redirected_command() {
    init_test_registry();

    let dispatcher = CommandDispatcher::new().expect("built-in commands register");
    let execute_and_creative = player_context_with_all([
        "minecraft.command.execute",
        "minecraft.command.gamemode.creative",
    ]);

    assert!(
        dispatcher
            .graph
            .parse("execute run gamemode creative", &execute_and_creative)
            .is_ok()
    );
}

#[test]
fn return_command_parse_shapes_match_vanilla() {
    init_test_registry();

    let dispatcher = CommandDispatcher::new().expect("built-in commands register");
    let return_player = player_context_with("minecraft.command.return");
    let return_and_seed =
        player_context_with_all(["minecraft.command.return", "minecraft.command.seed"]);

    let value = dispatcher
        .graph
        .parse("return 5", &return_player)
        .expect("return value parses");
    assert_eq!(value.path(), ["return", "value"]);

    let fail = dispatcher
        .graph
        .parse("return fail", &return_player)
        .expect("return fail parses");
    assert_eq!(fail.path(), ["return", "fail"]);

    let run = dispatcher
        .graph
        .parse("return run seed", &return_and_seed)
        .expect("return run parses");
    assert_eq!(run.path(), ["return", "run"]);
}

#[test]
fn return_run_requires_redirected_command_permission() {
    init_test_registry();

    let dispatcher = CommandDispatcher::new().expect("built-in commands register");
    let return_only = player_context_with("minecraft.command.return");

    assert!(
        dispatcher
            .graph
            .parse("return run gamemode creative", &return_only)
            .is_err()
    );
}

#[test]
fn return_run_uses_original_source_permissions_for_redirected_command() {
    init_test_registry();

    let dispatcher = CommandDispatcher::new().expect("built-in commands register");
    let return_and_creative = player_context_with_all([
        "minecraft.command.return",
        "minecraft.command.gamemode.creative",
    ]);

    assert!(
        dispatcher
            .graph
            .parse("return run gamemode creative", &return_and_creative)
            .is_ok()
    );
}

#[test]
fn derived_subcommand_permissions_use_root_permission_override() {
    let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
    let permission =
        super::command_permission_key(&minecraft, "long").expect("permission key parses");
    let mut dispatcher = CommandDispatcher::new_empty();

    let registration = CommandRegistration::new(
        literal("short").then(
            literal("child")
                .requires_subcommand_permission()
                .executes(|_, _| Ok(CommandResult::success())),
        ),
        minecraft,
    )
    .permission(permission)
    .alias("alias")
    .expect("alias parses");

    dispatcher
        .register_command(registration)
        .expect("command registers");

    let root_player = player_context_with("minecraft.command.long");
    assert!(dispatcher.graph.parse("short child", &root_player).is_ok());

    let child_player = player_context_with("minecraft.command.long.child");
    assert!(dispatcher.graph.parse("short child", &child_player).is_ok());
    assert!(dispatcher.graph.parse("alias child", &child_player).is_ok());
}

#[test]
fn additional_subcommand_permissions_require_root_and_child() {
    let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
    let mut dispatcher = CommandDispatcher::new_empty();

    let registration = CommandRegistration::new(
        literal("root")
            .then(
                literal("view")
                    .requires_subcommand_permission()
                    .executes(|_, _| Ok(CommandResult::success())),
            )
            .then(
                literal("admin")
                    .requires_additional_subcommand_permission()
                    .executes(|_, _| Ok(CommandResult::success())),
            ),
        minecraft,
    );

    dispatcher
        .register_command(registration)
        .expect("command registers");

    let root_player = player_context_with("minecraft.command.root");
    assert!(dispatcher.graph.has_root("root", &root_player));
    assert!(dispatcher.graph.parse("root admin", &root_player).is_err());

    let child_player = player_context_with("minecraft.command.root.admin");
    assert!(!dispatcher.graph.has_root("root", &child_player));

    let view_and_admin_player = player_context_with_all([
        "minecraft.command.root.view",
        "minecraft.command.root.admin",
    ]);
    assert!(dispatcher.graph.has_root("root", &view_and_admin_player));
    assert!(
        dispatcher
            .graph
            .parse("root view", &view_and_admin_player)
            .is_ok()
    );
    assert!(
        dispatcher
            .graph
            .parse("root admin", &view_and_admin_player)
            .is_err()
    );

    let admin_player =
        player_context_with_all(["minecraft.command.root", "minecraft.command.root.admin"]);
    assert!(dispatcher.graph.parse("root admin", &admin_player).is_ok());
}

#[test]
fn dispatcher_catalog_tracks_command_permissions() {
    init_test_registry();

    let dispatcher = CommandDispatcher::new().expect("built-in commands register");
    let suggestions = dispatcher
        .permission_catalog
        .suggestions("minecraft.command.gamemode");

    assert_eq!(
        suggestions,
        vec![
            "minecraft.command.gamemode".to_owned(),
            "minecraft.command.gamemode.adventure".to_owned(),
            "minecraft.command.gamemode.creative".to_owned(),
            "minecraft.command.gamemode.spectator".to_owned(),
            "minecraft.command.gamemode.survival".to_owned(),
        ]
    );
    assert_eq!(
        dispatcher
            .permission_catalog
            .suggestions(super::ENTITY_SELECTOR_PERMISSION_KEY),
        vec![
            super::ENTITY_SELECTOR_PERMISSION_KEY.to_owned(),
            super::ENTITY_SELECTOR_ADVANCED_PERMISSION_KEY.to_owned(),
        ]
    );
    assert!(
        dispatcher
            .permission_catalog
            .entries()
            .all(|entry| entry.sources().contains(&PermissionCatalogSource::Command))
    );
    assert!(
        !dispatcher
            .permission_catalog
            .suggestions("steel.command.sp")
            .iter()
            .any(|key| key == "steel.command.sp")
    );
}

#[test]
fn built_in_command_graph_has_no_validation_diagnostics() {
    init_test_registry();

    let dispatcher = CommandDispatcher::new().expect("built-in commands register");
    let validation = dispatcher.graph.validate();

    assert!(validation.is_empty());
}

#[test]
fn command_registration_rejects_ambiguous_graph_atomically() {
    let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
    let mut dispatcher = CommandDispatcher::new_empty();
    dispatcher
        .register_command(
            CommandRegistration::new(
                literal("other").executes(|_, _| Ok(CommandResult::success())),
                minecraft.clone(),
            )
            .public(),
        )
        .expect("existing command registers");

    let registration = CommandRegistration::new(
        literal("root").then_all([
            argument("value", StringParser::new(StringMode::SingleWord))
                .executes(|_, _| Ok(CommandResult::success())),
            literal("run").executes(|_, _| Ok(CommandResult::success())),
        ]),
        minecraft,
    )
    .public();

    let Err(error) = dispatcher.register_command(registration) else {
        panic!("ambiguous command should fail registration");
    };
    let CommandRegistrationError::InvalidGraphValidation(validation) = error else {
        panic!("expected graph validation error, got {error:?}");
    };
    assert_eq!(validation.ambiguities().len(), 1);

    let player = player_context();
    assert!(!dispatcher.graph.has_root("root", &player));
    assert!(dispatcher.graph.has_root("other", &player));
}

#[test]
fn aliases_validate_as_command_literals_not_permission_segments() {
    let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
    let mut dispatcher = CommandDispatcher::new_empty();
    let registration = CommandRegistration::new(
        literal("root").executes(|_, _| Ok(CommandResult::success())),
        minecraft,
    )
    .alias("Alias")
    .expect("alias is a valid command literal");

    dispatcher
        .register_command(registration)
        .expect("command registers");

    let player = player_context_with("minecraft.command.root");
    assert!(dispatcher.graph.has_root("Alias", &player));
}

#[test]
fn public_commands_do_not_silently_discard_derived_permission_markers() {
    let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
    let mut dispatcher = CommandDispatcher::new_empty();
    let registration = CommandRegistration::new(
        literal("root").then(literal("child").requires_subcommand_permission()),
        minecraft,
    )
    .public();

    let Err(error) = dispatcher.register_command(registration) else {
        panic!("public derived permission marker should fail registration");
    };

    assert!(matches!(
        error,
        CommandRegistrationError::InvalidGraph(
            CommandGraphError::UnresolvedDerivedPermission { .. }
        )
    ));
}

#[test]
fn public_commands_do_not_require_permission_segment_literals() {
    let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
    let mut dispatcher = CommandDispatcher::new_empty();
    let registration = CommandRegistration::new(
        literal("Visible").executes(|_, _| Ok(CommandResult::success())),
        minecraft,
    )
    .public();

    dispatcher
        .register_command(registration)
        .expect("public command registers without permission key derivation");

    let player = player_context();
    assert!(dispatcher.graph.has_root("Visible", &player));
    assert!(dispatcher.graph.parse("Visible", &player).is_ok());
}

#[test]
fn public_commands_register_explicit_permission_catalog_entries() {
    let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
    let admin_permission =
        PermissionKey::parse("steel.command.public.admin").expect("permission parses");
    let mut dispatcher = CommandDispatcher::new_empty();
    let registration = CommandRegistration::new(
        literal("public")
            .then(literal("open").executes(|_, _| Ok(CommandResult::success())))
            .then(
                literal("admin")
                    .requires_permission(admin_permission.clone())
                    .executes(|_, _| Ok(CommandResult::success())),
            ),
        minecraft,
    )
    .public();

    dispatcher
        .register_command(registration)
        .expect("public command registers");

    assert_eq!(
        dispatcher
            .permission_catalog
            .suggestions("steel.command.public"),
        vec!["steel.command.public.admin".to_owned()]
    );
    assert!(
        dispatcher
            .graph
            .parse("public open", &player_context())
            .is_ok()
    );
    assert!(
        dispatcher
            .graph
            .parse("public admin", &player_context())
            .is_err()
    );
    assert!(
        dispatcher
            .graph
            .parse(
                "public admin",
                &player_context_with("steel.command.public.admin"),
            )
            .is_ok()
    );
}

#[test]
fn aliases_reject_invalid_command_literals() {
    let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
    let Err(error) = CommandRegistration::new(literal("root"), minecraft).alias("bad alias") else {
        panic!("alias with whitespace should fail");
    };

    assert!(matches!(
        error,
        CommandRegistrationError::InvalidGraph(CommandGraphError::InvalidLiteralName { .. })
    ));
}

#[test]
fn failed_alias_registration_does_not_leave_primary_root() {
    let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
    let mut dispatcher = CommandDispatcher::new_empty();
    dispatcher
        .register_command(
            CommandRegistration::new(
                literal("other").executes(|_, _| Ok(CommandResult::success())),
                minecraft.clone(),
            )
            .public(),
        )
        .expect("existing command registers");

    let registration = CommandRegistration::new(
        literal("root").executes(|_, _| Ok(CommandResult::success())),
        minecraft,
    )
    .public()
    .alias("other")
    .expect("alias literal parses");
    let Err(error) = dispatcher.register_command(registration) else {
        panic!("colliding alias should reject registration");
    };

    assert!(matches!(
        error,
        CommandRegistrationError::InvalidGraph(CommandGraphError::LiteralCollision { .. })
    ));
    let player = player_context();
    assert!(!dispatcher.graph.has_root("root", &player));
    assert!(dispatcher.graph.has_root("other", &player));
}

#[test]
fn command_packet_filters_aliases_with_primary_permission() {
    let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
    let mut dispatcher = CommandDispatcher::new_empty();
    let registration = CommandRegistration::new(
        literal("primary").executes(|_, _| Ok(CommandResult::success())),
        minecraft,
    )
    .alias("alias")
    .expect("alias literal parses");

    dispatcher
        .register_command(registration)
        .expect("aliased command registers");

    let denied = dispatcher.get_commands(&player_context());
    assert!(root_literal_names(&denied).is_empty());

    let allowed = dispatcher.get_commands(&player_context_with("minecraft.command.primary"));
    assert_eq!(
        root_literal_names(&allowed),
        vec!["primary".to_owned(), "alias".to_owned()]
    );
}

#[test]
fn aliases_preserve_dynamic_argument_permission_base() {
    init_test_registry();
    let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
    let mut dispatcher = CommandDispatcher::new_empty();
    let registration =
        CommandRegistration::new(crate::command::commands::gamemode::command(), minecraft)
            .alias("gm")
            .expect("alias literal parses");

    dispatcher
        .register_command(registration)
        .expect("aliased gamemode command registers");

    let creative = player_context_with("minecraft.command.gamemode.creative");
    assert!(
        dispatcher
            .graph
            .parse("gamemode creative", &creative)
            .is_ok()
    );
    assert!(dispatcher.graph.parse("gm creative", &creative).is_ok());
    assert!(dispatcher.graph.parse("gm survival", &creative).is_err());
    assert!(
        dispatcher
            .graph
            .parse(
                "gm creative",
                &player_context_with("minecraft.command.gm.creative")
            )
            .is_err()
    );
}

#[test]
fn parse_error_mapping_uses_vanilla_integer_bound_order() {
    let error =
        super::CommandParseError::new(CommandParseErrorKind::IntegerTooLow { value: 1, min: 5 }, 5);

    let CommandError::Parse(report) =
        CommandDispatcher::parse_error_to_command_error("test 1", error)
    else {
        panic!("parse error should become structured feedback");
    };
    let feedback = report.into_feedback();
    let Content::Translate(message) = &feedback.primary.content else {
        panic!("primary message should be translated");
    };
    let args = message.args.as_ref().expect("integer low has arguments");

    assert_eq!(message.key.as_ref(), translations::ARGUMENT_INTEGER_LOW.0);
    assert_eq!(text_content(&args[0]), "5");
    assert_eq!(text_content(&args[1]), "1");
}

#[test]
fn parse_error_mapping_uses_vanilla_parser_translations() {
    assert_eq!(
        translation_key(&CommandDispatcher::parse_error_message(
            &CommandParseErrorKind::UnknownCommand
        )),
        translations::COMMAND_UNKNOWN_COMMAND.0
    );
    assert_eq!(
        translation_key(&CommandDispatcher::parse_error_message(
            &CommandParseErrorKind::TrailingData
        )),
        translations::COMMAND_EXPECTED_SEPARATOR.0
    );
    assert_eq!(
        translation_key(&CommandDispatcher::parse_error_message(
            &CommandParseErrorKind::InvalidBool("maybe".to_owned())
        )),
        translations::PARSING_BOOL_INVALID.0
    );
    assert_eq!(
        translation_key(&CommandDispatcher::parse_error_message(
            &CommandParseErrorKind::TooManyPlayers
        )),
        "argument.player.toomany"
    );
    assert_eq!(
        translation_key(&CommandDispatcher::parse_error_message(
            &CommandParseErrorKind::TooManyEntities
        )),
        "argument.entity.toomany"
    );
    assert_eq!(
        translation_key(&CommandDispatcher::parse_error_message(
            &CommandParseErrorKind::EntitiesNotAllowedForPlayerArgument
        )),
        "argument.player.entities"
    );
}

#[test]
fn parse_error_mapping_uses_brigadier_expected_number_messages() {
    assert_eq!(
        text_content(&CommandDispatcher::parse_error_message(
            &CommandParseErrorKind::ExpectedInteger
        )),
        "Expected integer"
    );
    assert_eq!(
        text_content(&CommandDispatcher::parse_error_message(
            &CommandParseErrorKind::ExpectedLong
        )),
        "Expected long"
    );
    assert_eq!(
        text_content(&CommandDispatcher::parse_error_message(
            &CommandParseErrorKind::ExpectedFloat
        )),
        "Expected float"
    );
    assert_eq!(
        text_content(&CommandDispatcher::parse_error_message(
            &CommandParseErrorKind::ExpectedDouble
        )),
        "Expected double"
    );
}

#[test]
fn parse_error_callback_rule_matches_brigadier_no_executable_only() {
    assert!(CommandDispatcher::parse_error_invokes_result_callback(
        &CommandParseErrorKind::IncompleteCommand
    ));

    let syntax_errors = [
        CommandParseErrorKind::EmptyCommand,
        CommandParseErrorKind::UnknownCommand,
        CommandParseErrorKind::ExpectedArgument,
        CommandParseErrorKind::TrailingData,
        CommandParseErrorKind::InvalidBool("maybe".to_owned()),
    ];
    for kind in syntax_errors {
        assert!(
            !CommandDispatcher::parse_error_invokes_result_callback(&kind),
            "{kind:?} should not invoke the result callback"
        );
    }
}

fn root_literal_names(commands: &super::CCommands) -> Vec<String> {
    let ProtocolCommandNode::Root { children } = &commands.nodes[commands.root_index as usize]
    else {
        panic!("root index should point at root node");
    };

    children
        .iter()
        .filter_map(|index| {
            let index = usize::try_from(*index).ok()?;
            match &commands.nodes[index] {
                ProtocolCommandNode::Literal { name, .. } => Some(name.to_string()),
                ProtocolCommandNode::Root { .. } | ProtocolCommandNode::Argument { .. } => None,
            }
        })
        .collect()
}
