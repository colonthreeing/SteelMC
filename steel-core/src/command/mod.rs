//! This module contains everything needed for commands (e.g., parsing, execution, and sender handling).
pub mod commands;
pub mod context;
pub mod error;
mod executor;
pub mod functions;
pub mod graph;
mod loot;
pub mod parsers;
pub mod reader;
pub mod requirement;
pub mod sender;
pub mod storage;
pub(crate) mod suggestions;

use steel_protocol::packets::game::{CCommandSuggestions, CCommands, CommandNode, SuggestionEntry};
use steel_utils::{Identifier, locks::SyncMutex, translations};
use text_components::TextComponent;
use text_components::translation::TranslatedMessage;

use crate::command::context::{CommandCallbackResult, CommandContext, CommandResultCallback};
use crate::command::error::CommandError;
use crate::command::functions::CommandFunction;
use crate::command::graph::{
    CommandExecutionStep, CommandGraph, CommandGraphError, CommandNodeBuilder, CommandParseError,
    CommandParseErrorKind, CommandResult, validate_command_node_name,
};
use crate::command::requirement::RequirementContext;
use crate::command::sender::CommandSender;
use crate::permission::{
    PermissionCatalog, PermissionCatalogSource, PermissionContextCatalog, PermissionExpr,
    PermissionKey, PermissionKeyError, PermissionMetadataCatalog, PermissionSegment,
};
use crate::player::Player;
use crate::server::Server;
use std::{borrow::Cow, collections::VecDeque, error::Error, fmt, sync::Arc};
use steel_registry::{
    game_rules::GameRuleValue,
    vanilla_game_rules::{MAX_COMMAND_FORKS, MAX_COMMAND_SEQUENCE_LENGTH},
};

pub(crate) use executor::CommandQueue;

pub(crate) struct CommandExecutionBudget {
    remaining: usize,
    limit: usize,
}

impl CommandExecutionBudget {
    fn for_context(context: &CommandContext) -> Self {
        Self::new(command_sequence_limit(context))
    }

    fn new(limit: usize) -> Self {
        let limit = limit.max(1);
        Self {
            remaining: limit,
            limit,
        }
    }

    pub(crate) fn consume(&mut self) -> Result<(), CommandError> {
        if self.remaining == 0 {
            return Err(CommandError::failure(format!(
                "Command execution stopped due to command sequence limit ({})",
                self.limit
            )));
        }

        self.remaining -= 1;
        Ok(())
    }
}

fn command_sequence_limit(context: &CommandContext) -> usize {
    match context.world.get_game_rule(&MAX_COMMAND_SEQUENCE_LENGTH) {
        GameRuleValue::Int(value) => value.max(1) as usize,
        GameRuleValue::Bool(_) => default_command_sequence_limit(),
    }
}

fn default_command_sequence_limit() -> usize {
    match MAX_COMMAND_SEQUENCE_LENGTH.default_value {
        GameRuleValue::Int(value) => value.max(1) as usize,
        GameRuleValue::Bool(_) => 1,
    }
}

fn command_fork_limit(context: &CommandContext) -> usize {
    match context.world.get_game_rule(&MAX_COMMAND_FORKS) {
        GameRuleValue::Int(value) => value.max(0) as usize,
        GameRuleValue::Bool(_) => default_command_fork_limit(),
    }
}

fn default_command_fork_limit() -> usize {
    match MAX_COMMAND_FORKS.default_value {
        GameRuleValue::Int(value) => value.max(0) as usize,
        GameRuleValue::Bool(_) => 0,
    }
}

fn check_command_fork_limit(
    existing_stage_contexts: usize,
    new_stage_contexts: usize,
    limit: usize,
) -> Result<(), CommandError> {
    if fork_limit_reached(existing_stage_contexts, new_stage_contexts, limit) {
        return Err(command_fork_limit_error(limit));
    }
    Ok(())
}

fn fork_limit_reached(
    existing_stage_contexts: usize,
    new_stage_contexts: usize,
    limit: usize,
) -> bool {
    existing_stage_contexts.saturating_add(new_stage_contexts) >= limit
}

fn command_fork_limit_error(limit: usize) -> CommandError {
    CommandError::failure(TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed("command.forkLimit"),
        fallback: None,
        args: Some(Box::new([TextComponent::from(limit.to_string())])),
    }))
}

/// Parses and dispatches commands through the command graph.
#[derive(Clone, Default)]
pub struct CommandDispatcher {
    /// Dynamic command graph.
    graph: CommandGraph,
    permission_catalog: PermissionCatalog,
    permission_metadata_catalog: PermissionMetadataCatalog,
    permission_context_catalog: PermissionContextCatalog,
}

pub(crate) struct CommandRegistration {
    root: CommandNodeBuilder,
    namespace: PermissionSegment,
    permission: CommandPermissionMode,
    aliases: Vec<String>,
}

impl CommandRegistration {
    pub(crate) const fn new(root: CommandNodeBuilder, namespace: PermissionSegment) -> Self {
        Self {
            root,
            namespace,
            permission: CommandPermissionMode::Auto,
            aliases: Vec::new(),
        }
    }

    pub(crate) fn minecraft(root: CommandNodeBuilder) -> Result<Self, CommandRegistrationError> {
        Ok(Self::new(root, PermissionSegment::parse("minecraft")?))
    }

    pub(crate) fn steel(root: CommandNodeBuilder) -> Result<Self, CommandRegistrationError> {
        Ok(Self::new(root, PermissionSegment::parse("steel")?))
    }

    pub(crate) fn public(mut self) -> Self {
        self.permission = CommandPermissionMode::Public;
        self
    }

    pub(crate) fn permission(mut self, permission: PermissionKey) -> Self {
        self.permission = CommandPermissionMode::Override(permission);
        self
    }

    pub(crate) fn permission_base(self, command: &str) -> Result<Self, CommandRegistrationError> {
        let permission = command_permission_key(&self.namespace, command)?;
        Ok(self.permission(permission))
    }

    pub(crate) fn alias(mut self, alias: &str) -> Result<Self, CommandRegistrationError> {
        validate_command_node_name(alias).map_err(|source| {
            CommandGraphError::InvalidLiteralName {
                name: alias.to_owned(),
                source,
            }
        })?;
        self.aliases.push(alias.to_owned());
        Ok(self)
    }

    fn resolved_permission_base(&self) -> Result<Option<PermissionKey>, CommandRegistrationError> {
        match &self.permission {
            CommandPermissionMode::Public => Ok(None),
            CommandPermissionMode::Auto => {
                let command_name = self
                    .root
                    .literal_name()
                    .ok_or(CommandRegistrationError::RootMustBeLiteral)?;
                Ok(Some(command_permission_key(&self.namespace, command_name)?))
            }
            CommandPermissionMode::Override(permission) => Ok(Some(permission.clone())),
        }
    }
}

/// Static registration metadata for one built-in command module.
#[derive(Clone, Copy)]
pub(crate) struct CommandRegistrationSpec {
    namespace: CommandRegistrationNamespace,
    permission: CommandRegistrationSpecPermission,
    aliases: &'static [&'static str],
}

impl CommandRegistrationSpec {
    pub(crate) const fn minecraft() -> Self {
        Self::new(CommandRegistrationNamespace::Minecraft)
    }

    pub(crate) const fn steel() -> Self {
        Self::new(CommandRegistrationNamespace::Steel)
    }

    const fn new(namespace: CommandRegistrationNamespace) -> Self {
        Self {
            namespace,
            permission: CommandRegistrationSpecPermission::Auto,
            aliases: &[],
        }
    }

    pub(crate) const fn public(mut self) -> Self {
        self.permission = CommandRegistrationSpecPermission::Public;
        self
    }

    pub(crate) const fn permission_base(mut self, command: &'static str) -> Self {
        self.permission = CommandRegistrationSpecPermission::PermissionBase(command);
        self
    }

    pub(crate) const fn aliases(mut self, aliases: &'static [&'static str]) -> Self {
        self.aliases = aliases;
        self
    }

    fn register(
        self,
        root: CommandNodeBuilder,
    ) -> Result<CommandRegistration, CommandRegistrationError> {
        let registration = match self.namespace {
            CommandRegistrationNamespace::Minecraft => CommandRegistration::minecraft(root)?,
            CommandRegistrationNamespace::Steel => CommandRegistration::steel(root)?,
        };
        let mut registration = match self.permission {
            CommandRegistrationSpecPermission::Auto => registration,
            CommandRegistrationSpecPermission::Public => registration.public(),
            CommandRegistrationSpecPermission::PermissionBase(command) => {
                registration.permission_base(command)?
            }
        };
        for alias in self.aliases {
            registration = registration.alias(alias)?;
        }
        Ok(registration)
    }
}

#[derive(Clone, Copy)]
enum CommandRegistrationNamespace {
    Minecraft,
    Steel,
}

#[derive(Clone, Copy)]
enum CommandRegistrationSpecPermission {
    Auto,
    Public,
    PermissionBase(&'static str),
}

enum CommandPermissionMode {
    Auto,
    Public,
    Override(PermissionKey),
}

/// Invalid command registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandRegistrationError {
    /// A command root was not a literal node.
    RootMustBeLiteral,
    /// A command or alias produced an invalid permission key.
    InvalidPermissionKey(PermissionKeyError),
    /// The command graph rejected a node registration.
    InvalidGraph(CommandGraphError),
    /// Command graph validation found diagnostics after registration.
    InvalidGraphValidation(crate::command::graph::CommandGraphValidation),
}

impl fmt::Display for CommandRegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RootMustBeLiteral => write!(f, "command root must be a literal node"),
            Self::InvalidPermissionKey(error) => write!(f, "{error}"),
            Self::InvalidGraph(error) => write!(f, "{error}"),
            Self::InvalidGraphValidation(validation) => {
                write!(f, "command graph validation failed")?;
                if let Some(ambiguity) = validation.ambiguities().first() {
                    write!(
                        f,
                        ": ambiguous child '{}' and sibling '{}' under '{}'",
                        ambiguity.child,
                        ambiguity.sibling,
                        ambiguity.parent_path.join(" ")
                    )?;
                }
                Ok(())
            }
        }
    }
}

impl Error for CommandRegistrationError {}

impl From<PermissionKeyError> for CommandRegistrationError {
    fn from(value: PermissionKeyError) -> Self {
        Self::InvalidPermissionKey(value)
    }
}

impl From<CommandGraphError> for CommandRegistrationError {
    fn from(value: CommandGraphError) -> Self {
        Self::InvalidGraph(value)
    }
}

fn command_permission_key(
    namespace: &PermissionSegment,
    command: &str,
) -> Result<PermissionKey, PermissionKeyError> {
    PermissionKey::from_segments([
        namespace.clone(),
        PermissionSegment::parse("command")?,
        PermissionSegment::parse(command)?,
    ])
}

pub(crate) fn minecraft_command_permission_key(
    command: &str,
) -> Result<PermissionKey, PermissionKeyError> {
    command_permission_key(&PermissionSegment::parse("minecraft")?, command)
}

pub(crate) const ENTITY_SELECTOR_PERMISSION_KEY: &str = "minecraft.command.selector";
pub(crate) const ENTITY_SELECTOR_ADVANCED_PERMISSION_KEY: &str =
    "minecraft.command.selector.advanced";

pub(crate) fn entity_selector_permission_key() -> Result<PermissionKey, PermissionKeyError> {
    PermissionKey::parse(ENTITY_SELECTOR_PERMISSION_KEY)
}

pub(crate) fn entity_selector_advanced_permission_key() -> Result<PermissionKey, PermissionKeyError>
{
    PermissionKey::parse(ENTITY_SELECTOR_ADVANCED_PERMISSION_KEY)
}

pub(crate) fn entity_selector_permission_expr() -> Result<PermissionExpr, PermissionKeyError> {
    Ok(PermissionExpr::key(entity_selector_permission_key()?))
}

pub(crate) fn entity_selector_advanced_permission_expr()
-> Result<PermissionExpr, PermissionKeyError> {
    Ok(PermissionExpr::key(
        entity_selector_advanced_permission_key()?,
    ))
}

impl CommandDispatcher {
    /// Creates a new command dispatcher with built-in commands.
    ///
    /// # Errors
    ///
    /// Returns an error when a built-in command registration is invalid.
    pub fn new() -> Result<Self, CommandRegistrationError> {
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
            dispatcher.register_command(registration)?;
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

    fn register_command(
        &mut self,
        registration: CommandRegistration,
    ) -> Result<(), CommandRegistrationError> {
        let permission_base = registration.resolved_permission_base()?;
        let mut command_catalog = PermissionCatalog::new();
        let root = if let Some(permission_base) = &permission_base {
            command_catalog.insert(permission_base.clone(), PermissionCatalogSource::Command);
            registration
                .root
                .clone()
                .resolve_subcommand_permissions(&permission_base, &mut command_catalog)?
        } else {
            registration.root.clone()
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

    /// Executes a command using an existing command context.
    pub fn dispatch_with_context(
        &self,
        command: String,
        context: &mut CommandContext,
    ) -> Result<CommandResult, CommandError> {
        let mut budget = CommandExecutionBudget::for_context(context);
        self.dispatch_with_budget(command, context, &mut budget)
    }

    pub(crate) fn dispatch_with_budget(
        &self,
        command: String,
        context: &mut CommandContext,
        budget: &mut CommandExecutionBudget,
    ) -> Result<CommandResult, CommandError> {
        Ok(self
            .dispatch_active_with_queue(
                ActiveCommand::Borrowed { command, context },
                VecDeque::new(),
                budget,
            )?
            .result)
    }

    pub(crate) fn run_functions_for_condition(
        &self,
        functions: &[CommandFunction],
        context: &CommandContext,
        budget: &mut CommandExecutionBudget,
    ) -> Result<CommandFunctionConditionResult, CommandError> {
        if functions.is_empty() {
            return Ok(CommandFunctionConditionResult::NoFunctions);
        }

        let function_context = context
            .clone()
            .without_result_callbacks()
            .with_suppressed_output();
        let mut queue = VecDeque::new();
        let function_frame = QueuedFrame::new(1, 1);
        for function in functions {
            queue.push_back(QueuedAction::FunctionCall(QueuedFunctionCall::new(
                function.clone(),
                function_context.clone(),
                function_frame.clone(),
                CommandResultCallback::empty(),
                true,
                true,
            )));
        }
        queue.push_back(QueuedAction::Fallthrough(function_frame));

        let mut frame_return = None;
        let Some(first) = next_queued_command(&mut queue, budget, &mut frame_return)? else {
            return Ok(frame_return.map_or(
                CommandFunctionConditionResult::NoFunctions,
                CommandFunctionConditionResult::Callback,
            ));
        };

        let outcome = self
            .dispatch_active_with_queue(ActiveCommand::Owned(first), queue, budget)?
            .frame_return;
        Ok(outcome.map_or(
            CommandFunctionConditionResult::NoFunctions,
            CommandFunctionConditionResult::Callback,
        ))
    }

    fn dispatch_active_with_queue(
        &self,
        mut active: ActiveCommand<'_>,
        mut queue: VecDeque<QueuedAction>,
        budget: &mut CommandExecutionBudget,
    ) -> Result<DispatchOutcome, CommandError> {
        let fork_limit = command_fork_limit(active.context());
        let mut total_success_count = 0_i32;
        let mut last_result = 0_i32;
        let mut completed_forked_context = false;
        let mut frame_return = None;

        loop {
            budget.consume()?;
            let active_command = active.command().to_owned();
            let step = self.execute_command_step(&active_command, active.context_mut(), budget);
            let step = match step {
                Ok(step) => step,
                Err(_error) if active.continues_after_command_error() => {
                    let Some(next) = next_queued_command(&mut queue, budget, &mut frame_return)?
                    else {
                        return Ok(dispatch_outcome(
                            total_success_count,
                            last_result,
                            completed_forked_context,
                            frame_return,
                        ));
                    };
                    active = ActiveCommand::Owned(next);
                    continue;
                }
                Err(error) => return Err(error),
            };

            match step {
                CommandExecutionStep::Complete(mut result) => {
                    if active.is_returning() && !result.returns_from_frame() {
                        result = result.as_return_success();
                    }
                    let returns_from_frame = result.returns_from_frame();
                    let active_frame = active.frame();
                    let callback_result = command_callback_result(result);
                    active.context_mut().on_command_result(callback_result);
                    if active.is_forked() {
                        completed_forked_context = true;
                    }
                    last_result = result.return_value();
                    total_success_count = total_success_count
                        .saturating_add(execution_success_count(result, active.is_forked()));
                    if returns_from_frame {
                        return_from_frame(
                            &mut queue,
                            &active_frame,
                            callback_result,
                            &mut frame_return,
                        );
                    }
                    let Some(next) = next_queued_command(&mut queue, budget, &mut frame_return)?
                    else {
                        return Ok(dispatch_outcome(
                            total_success_count,
                            last_result,
                            completed_forked_context,
                            frame_return,
                        ));
                    };
                    active = ActiveCommand::Owned(next);
                }
                CommandExecutionStep::CallFunctions { functions } => {
                    let active_frame = active.frame();
                    let return_parent_frame = active.is_returning();
                    let output_suppressed = active.context().is_output_suppressed();
                    let original_sender = active.context().sender.clone();
                    let original_callback = active.context().result_callback();
                    let function_context = active
                        .context()
                        .clone()
                        .without_result_callbacks()
                        .with_suppressed_output();
                    let mut actions = Vec::with_capacity(
                        functions
                            .len()
                            .saturating_add(usize::from(return_parent_frame))
                            .saturating_add(usize::from(
                                functions.len() > 1 && !return_parent_frame,
                            )),
                    );
                    if return_parent_frame {
                        let function_return_callback =
                            original_callback.chain(active_frame.return_callback());
                        for function in functions {
                            let return_callback = decorated_function_callback(
                                original_sender.clone(),
                                output_suppressed,
                                function.id().clone(),
                                function_return_callback.clone(),
                            );
                            actions.push(QueuedAction::FunctionCall(QueuedFunctionCall::new(
                                function,
                                function_context.clone(),
                                active_frame.clone(),
                                return_callback,
                                true,
                                true,
                            )));
                        }
                        actions.push(QueuedAction::Fallthrough(active_frame));
                    } else if functions.len() > 1 {
                        let accumulator =
                            Arc::new(SyncMutex::new(FunctionReturnAccumulator::new()));
                        let function_return_callback =
                            accumulating_function_callback(Arc::clone(&accumulator));
                        for function in functions {
                            let return_callback = decorated_function_callback(
                                original_sender.clone(),
                                output_suppressed,
                                function.id().clone(),
                                function_return_callback.clone(),
                            );
                            actions.push(QueuedAction::FunctionCall(QueuedFunctionCall::new(
                                function,
                                function_context.clone(),
                                active_frame.clone(),
                                return_callback,
                                false,
                                false,
                            )));
                        }
                        actions.push(QueuedAction::Callback(QueuedCallbackAction::new(
                            active_frame,
                            move || {
                                let accumulator = accumulator.lock();
                                if accumulator.any_result {
                                    original_callback.on_result(CommandCallbackResult {
                                        success: true,
                                        result: accumulator.sum,
                                    });
                                }
                            },
                        )));
                    } else {
                        for function in functions {
                            let return_callback = decorated_function_callback(
                                original_sender.clone(),
                                output_suppressed,
                                function.id().clone(),
                                original_callback.clone(),
                            );
                            actions.push(QueuedAction::FunctionCall(QueuedFunctionCall::new(
                                function,
                                function_context.clone(),
                                active_frame.clone(),
                                return_callback,
                                false,
                                false,
                            )));
                        }
                    }
                    queue_next_actions(&mut queue, actions);
                    let Some(next) = next_queued_command(&mut queue, budget, &mut frame_return)?
                    else {
                        return Ok(dispatch_outcome(
                            total_success_count,
                            last_result,
                            completed_forked_context,
                            frame_return,
                        ));
                    };
                    active = ActiveCommand::Owned(next);
                }
                CommandExecutionStep::Redirect {
                    command: next_command,
                    mut contexts,
                    forked,
                    returns,
                } => {
                    let next_forked = active.is_forked() || forked;
                    let next_fork_stage = active.next_fork_stage(forked);
                    let next_returning = active.is_returning() || returns;
                    let next_frame = active.frame();
                    if active.is_forked() && !returns {
                        self.collect_redirect_sibling_contexts(
                            RedirectSiblingBatch {
                                active_command: &active_command,
                                active_fork_stage: active.fork_stage(),
                                active_returning: active.is_returning(),
                                active_frame: &next_frame,
                                next_command: &next_command,
                                forked,
                                returns,
                            },
                            &mut contexts,
                            &mut queue,
                            budget,
                        )?;
                    }
                    if returns {
                        discard_command_frame(&mut queue, next_frame.return_discard_depth);
                    }
                    if forked {
                        check_command_fork_limit(
                            queued_fork_stage_contexts(&queue, next_fork_stage, &next_command),
                            contexts.len(),
                            fork_limit,
                        )?;
                    }
                    queue_next_actions(
                        &mut queue,
                        redirect_actions(
                            next_command,
                            contexts,
                            next_forked,
                            next_fork_stage,
                            next_returning,
                            next_frame,
                        ),
                    );
                    let Some(next) = next_queued_command(&mut queue, budget, &mut frame_return)?
                    else {
                        return Ok(dispatch_outcome(
                            total_success_count,
                            last_result,
                            completed_forked_context,
                            frame_return,
                        ));
                    };
                    active = ActiveCommand::Owned(next);
                }
            }
        }
    }

    fn execute_command_step(
        &self,
        command: &str,
        context: &mut CommandContext,
        budget: &mut CommandExecutionBudget,
    ) -> Result<CommandExecutionStep, CommandError> {
        match self.graph.parse(command, context) {
            Ok(parsed) => parsed.execute_step(context, budget).map_err(|error| {
                if parsed.invokes_result_callback_on_error() {
                    context.on_command_result(CommandCallbackResult {
                        success: false,
                        result: 0,
                    });
                }
                error
            }),
            Err(error) => {
                context.on_command_result(CommandCallbackResult {
                    success: false,
                    result: 0,
                });
                Err(Self::parse_error_to_command_error(command, error))
            }
        }
    }

    fn collect_redirect_sibling_contexts(
        &self,
        batch: RedirectSiblingBatch<'_>,
        contexts: &mut Vec<CommandContext>,
        queue: &mut VecDeque<QueuedAction>,
        budget: &mut CommandExecutionBudget,
    ) -> Result<(), CommandError> {
        for mut sibling in drain_redirect_sibling_commands(queue, &batch) {
            budget.consume()?;
            let step = self.execute_command_step(&sibling.command, &mut sibling.context, budget);
            match step {
                Ok(CommandExecutionStep::Redirect {
                    command,
                    contexts: sibling_contexts,
                    forked,
                    returns,
                }) if command == batch.next_command
                    && forked == batch.forked
                    && returns == batch.returns =>
                {
                    contexts.extend(sibling_contexts);
                }
                Ok(_) => {
                    return Err(CommandError::InvalidConsumption(Some(
                        "batched redirect sibling produced a different command step".to_owned(),
                    )));
                }
                Err(_error) if sibling.continues_after_command_error() => {}
                Err(error) => return Err(error),
            }
        }

        Ok(())
    }

    pub(crate) fn parse_error_to_command_error(
        input: &str,
        error: CommandParseError,
    ) -> CommandError {
        let cursor = error.cursor();
        CommandError::parse(Self::parse_error_message(error.kind()), input, cursor)
    }

    fn parse_error_message(kind: &CommandParseErrorKind) -> TextComponent {
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
            CommandParseErrorKind::InvalidInteger(value) => translations::PARSING_INT_INVALID
                .message([TextComponent::from(value.clone())])
                .into(),
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
            CommandParseErrorKind::InvalidEntity(_) => {
                TextComponent::from(&translations::ARGUMENT_ENTITY_NOTFOUND_ENTITY)
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

enum ActiveCommand<'a> {
    Borrowed {
        command: String,
        context: &'a mut CommandContext,
    },
    Owned(QueuedCommand),
}

struct QueuedCommand {
    command: String,
    context: CommandContext,
    forked: bool,
    fork_stage: usize,
    returning: bool,
    frame: QueuedFrame,
}

impl QueuedCommand {
    fn new(
        command: String,
        context: CommandContext,
        forked: bool,
        fork_stage: usize,
        returning: bool,
        frame: QueuedFrame,
    ) -> Self {
        Self {
            command,
            context,
            forked,
            fork_stage,
            returning,
            frame,
        }
    }

    fn is_redirect_sibling(&self, batch: &RedirectSiblingBatch<'_>) -> bool {
        redirect_sibling_matches(
            &self.command,
            self.fork_stage,
            self.returning,
            &self.frame,
            batch,
        )
    }

    const fn continues_after_command_error(&self) -> bool {
        self.forked || self.frame.depth > 0
    }
}

enum QueuedAction {
    Command(QueuedCommand),
    FunctionCall(QueuedFunctionCall),
    Fallthrough(QueuedFrame),
    Callback(QueuedCallbackAction),
}

impl QueuedAction {
    fn frame_depth(&self) -> usize {
        match self {
            Self::Command(command) => command.frame.depth,
            Self::FunctionCall(call) => call.frame.depth,
            Self::Fallthrough(frame) => frame.depth,
            Self::Callback(callback) => callback.frame.depth,
        }
    }
}

struct QueuedFunctionCall {
    function: CommandFunction,
    context: CommandContext,
    frame: QueuedFrame,
    return_callback: CommandResultCallback,
    return_parent_frame: bool,
    propagates_return: bool,
}

impl QueuedFunctionCall {
    fn new(
        function: CommandFunction,
        context: CommandContext,
        frame: QueuedFrame,
        return_callback: CommandResultCallback,
        return_parent_frame: bool,
        propagates_return: bool,
    ) -> Self {
        Self {
            function,
            context,
            frame,
            return_callback,
            return_parent_frame,
            propagates_return,
        }
    }

    fn enqueue_commands(self, queue: &mut VecDeque<QueuedAction>) {
        let child_frame = if self.return_parent_frame {
            QueuedFrame::with_return_callback(
                self.frame.depth.saturating_add(1),
                self.frame.return_discard_depth,
                self.return_callback,
                self.propagates_return,
            )
        } else {
            QueuedFrame::with_return_callback(
                self.frame.depth.saturating_add(1),
                self.frame.depth.saturating_add(1),
                self.return_callback,
                self.propagates_return,
            )
        };
        for command in self.function.commands().iter().rev() {
            queue.push_front(QueuedAction::Command(QueuedCommand::new(
                command.clone(),
                self.context.clone(),
                false,
                0,
                false,
                child_frame.clone(),
            )));
        }
    }
}

struct QueuedCallbackAction {
    frame: QueuedFrame,
    callback: Box<dyn Fn() + Send + Sync>,
}

impl QueuedCallbackAction {
    fn new(frame: QueuedFrame, callback: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            frame,
            callback: Box::new(callback),
        }
    }

    fn run(self) {
        (self.callback)();
    }
}

#[derive(Clone)]
struct QueuedFrame {
    depth: usize,
    return_discard_depth: usize,
    return_callback: CommandResultCallback,
    propagates_return: bool,
}

impl QueuedFrame {
    fn new(depth: usize, return_discard_depth: usize) -> Self {
        Self::with_return_callback(
            depth,
            return_discard_depth,
            CommandResultCallback::empty(),
            true,
        )
    }

    fn with_return_callback(
        depth: usize,
        return_discard_depth: usize,
        return_callback: CommandResultCallback,
        propagates_return: bool,
    ) -> Self {
        Self {
            depth,
            return_discard_depth,
            return_callback,
            propagates_return,
        }
    }

    fn for_depth(depth: usize) -> Self {
        Self::new(depth, depth)
    }

    fn return_callback(&self) -> CommandResultCallback {
        self.return_callback.clone()
    }

    fn on_return(&self, result: CommandCallbackResult) {
        self.return_callback.on_result(result);
    }

    const fn propagates_return(&self) -> bool {
        self.propagates_return
    }
}

struct DispatchOutcome {
    result: CommandResult,
    frame_return: Option<CommandCallbackResult>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommandFunctionConditionResult {
    NoFunctions,
    Callback(CommandCallbackResult),
}

struct FunctionReturnAccumulator {
    any_result: bool,
    sum: i32,
}

impl FunctionReturnAccumulator {
    const fn new() -> Self {
        Self {
            any_result: false,
            sum: 0,
        }
    }

    fn add(&mut self, result: i32) {
        self.any_result = true;
        self.sum = self.sum.wrapping_add(result);
    }
}

fn accumulating_function_callback(
    accumulator: Arc<SyncMutex<FunctionReturnAccumulator>>,
) -> CommandResultCallback {
    CommandResultCallback::new(move |result| {
        accumulator.lock().add(result.result);
    })
}

fn decorated_function_callback(
    sender: CommandSender,
    output_suppressed: bool,
    function_id: Identifier,
    callback: CommandResultCallback,
) -> CommandResultCallback {
    if output_suppressed {
        return callback;
    }

    CommandResultCallback::new(move |result| {
        sender.send_message(&function_result_message(&function_id, result.result));
        callback.on_result(result);
    })
}

fn function_result_message(function_id: &Identifier, result: i32) -> TextComponent {
    TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed("commands.function.result"),
        fallback: None,
        args: Some(Box::new([
            TextComponent::from(function_id.to_string()),
            TextComponent::from(result.to_string()),
        ])),
    })
}

impl ActiveCommand<'_> {
    fn command(&self) -> &str {
        match self {
            Self::Borrowed { command, .. } => command,
            Self::Owned(QueuedCommand { command, .. }) => command,
        }
    }

    fn context(&self) -> &CommandContext {
        match self {
            Self::Borrowed { context, .. } => context,
            Self::Owned(QueuedCommand { context, .. }) => context,
        }
    }

    fn context_mut(&mut self) -> &mut CommandContext {
        match self {
            Self::Borrowed { context, .. } => context,
            Self::Owned(QueuedCommand { context, .. }) => context,
        }
    }

    const fn is_forked(&self) -> bool {
        match self {
            Self::Borrowed { .. } => false,
            Self::Owned(QueuedCommand { forked, .. }) => *forked,
        }
    }

    const fn is_returning(&self) -> bool {
        match self {
            Self::Borrowed { .. } => false,
            Self::Owned(QueuedCommand { returning, .. }) => *returning,
        }
    }

    fn fork_stage(&self) -> usize {
        match self {
            Self::Borrowed { .. } => 0,
            Self::Owned(QueuedCommand { fork_stage, .. }) => *fork_stage,
        }
    }

    fn frame(&self) -> QueuedFrame {
        match self {
            Self::Borrowed { .. } => QueuedFrame::for_depth(0),
            Self::Owned(QueuedCommand { frame, .. }) => frame.clone(),
        }
    }

    fn frame_depth(&self) -> usize {
        self.frame().depth
    }

    fn next_fork_stage(&self, forked: bool) -> usize {
        if forked {
            self.fork_stage().saturating_add(1)
        } else {
            self.fork_stage()
        }
    }

    fn continues_after_command_error(&self) -> bool {
        self.is_forked() || self.frame_depth() > 0
    }
}

struct RedirectSiblingBatch<'a> {
    active_command: &'a str,
    active_fork_stage: usize,
    active_returning: bool,
    active_frame: &'a QueuedFrame,
    next_command: &'a str,
    forked: bool,
    returns: bool,
}

fn queue_next_actions(
    queue: &mut VecDeque<QueuedAction>,
    actions: impl IntoIterator<Item = QueuedAction>,
) {
    let actions = actions.into_iter().collect::<Vec<_>>();
    for action in actions.into_iter().rev() {
        queue.push_front(action);
    }
}

fn redirect_actions(
    command: String,
    contexts: Vec<CommandContext>,
    forked: bool,
    fork_stage: usize,
    returning: bool,
    frame: QueuedFrame,
) -> Vec<QueuedAction> {
    if contexts.is_empty() && returning {
        return vec![QueuedAction::Fallthrough(frame)];
    }

    contexts
        .into_iter()
        .map(|context| {
            QueuedAction::Command(QueuedCommand::new(
                command.clone(),
                context,
                forked,
                fork_stage,
                returning,
                frame.clone(),
            ))
        })
        .collect()
}

fn drain_redirect_sibling_commands(
    queue: &mut VecDeque<QueuedAction>,
    batch: &RedirectSiblingBatch<'_>,
) -> Vec<QueuedCommand> {
    let mut siblings = Vec::new();
    while queue.front().is_some_and(|queued| {
        matches!(queued, QueuedAction::Command(command) if command.is_redirect_sibling(batch))
    }) {
        match queue.pop_front() {
            Some(QueuedAction::Command(command)) => siblings.push(command),
            _ => break,
        }
    }
    siblings
}

fn redirect_sibling_matches(
    command: &str,
    fork_stage: usize,
    returning: bool,
    frame: &QueuedFrame,
    batch: &RedirectSiblingBatch<'_>,
) -> bool {
    command == batch.active_command
        && fork_stage == batch.active_fork_stage
        && returning == batch.active_returning
        && same_frame_return_scope(frame, batch.active_frame)
}

const fn same_frame_return_scope(left: &QueuedFrame, right: &QueuedFrame) -> bool {
    left.depth == right.depth
        && left.return_discard_depth == right.return_discard_depth
        && left.propagates_return == right.propagates_return
}

fn return_from_frame(
    queue: &mut VecDeque<QueuedAction>,
    frame: &QueuedFrame,
    result: CommandCallbackResult,
    frame_return: &mut Option<CommandCallbackResult>,
) {
    frame.on_return(result);
    if frame.propagates_return() {
        *frame_return = Some(result);
    }
    discard_command_frame(queue, frame.return_discard_depth);
}

fn next_queued_command(
    queue: &mut VecDeque<QueuedAction>,
    budget: &mut CommandExecutionBudget,
    frame_return: &mut Option<CommandCallbackResult>,
) -> Result<Option<QueuedCommand>, CommandError> {
    loop {
        let Some(action) = queue.pop_front() else {
            return Ok(None);
        };
        match action {
            QueuedAction::Command(command) => return Ok(Some(command)),
            QueuedAction::FunctionCall(call) => {
                budget.consume()?;
                call.enqueue_commands(queue);
            }
            QueuedAction::Fallthrough(frame) => {
                let result = CommandCallbackResult {
                    success: false,
                    result: 0,
                };
                return_from_frame(queue, &frame, result, frame_return);
            }
            QueuedAction::Callback(callback) => {
                callback.run();
            }
        }
    }
}

fn discard_command_frame(queue: &mut VecDeque<QueuedAction>, frame_depth: usize) {
    while queue
        .front()
        .is_some_and(|queued| queued.frame_depth() >= frame_depth)
    {
        queue.pop_front();
    }
}

fn queued_fork_stage_contexts(
    queue: &VecDeque<QueuedAction>,
    fork_stage: usize,
    command: &str,
) -> usize {
    queue
        .iter()
        .filter(|queued| {
            matches!(
                queued,
                QueuedAction::Command(queued)
                    if queued.fork_stage == fork_stage && queued.command == command
            )
        })
        .count()
}

fn execution_success_count(result: CommandResult, forked: bool) -> i32 {
    if forked { 1 } else { result.return_value() }
}

fn command_callback_result(result: CommandResult) -> CommandCallbackResult {
    CommandCallbackResult {
        success: result.callback_success(),
        result: result.return_value(),
    }
}

fn dispatch_result(
    total_success_count: i32,
    last_result: i32,
    completed_forked_context: bool,
) -> CommandResult {
    CommandResult::from_return_value(if completed_forked_context {
        total_success_count
    } else {
        last_result
    })
}

fn dispatch_outcome(
    total_success_count: i32,
    last_result: i32,
    completed_forked_context: bool,
    frame_return: Option<CommandCallbackResult>,
) -> DispatchOutcome {
    let result = frame_return.map_or_else(
        || dispatch_result(total_success_count, last_result, completed_forked_context),
        command_result_from_callback,
    );
    DispatchOutcome {
        result,
        frame_return,
    }
}

fn command_result_from_callback(callback: CommandCallbackResult) -> CommandResult {
    if callback.success {
        CommandResult::return_success(callback.result)
    } else {
        CommandResult::return_failure()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CommandCallbackResult, CommandDispatcher, CommandExecutionBudget, CommandRegistration,
        CommandRegistrationError,
    };
    use crate::command::{
        context::CommandResultCallback,
        error::CommandError,
        graph::{
            CommandGraphError, CommandParseErrorKind, CommandResult, StringParser, argument,
            literal,
        },
        reader::StringMode,
        requirement::{CommandInputContext, CommandSourceKind, PermissionExpr, RequirementContext},
        sender::CommandSender,
    };
    use crate::permission::{
        PermissionCatalogSource, PermissionEntry, PermissionKey, PermissionSegment, PermissionSet,
    };
    use steel_registry::test_support::init_test_registry;
    use steel_utils::{Identifier, translations};
    use text_components::{TextComponent, content::Content};

    struct TestContext {
        permissions: PermissionSet,
    }

    impl RequirementContext for TestContext {
        fn source_kind(&self) -> CommandSourceKind {
            CommandSourceKind::Player
        }

        fn has_permission(&self, permission: &PermissionExpr) -> bool {
            self.permissions.allows(permission)
        }
    }

    impl CommandInputContext for TestContext {}

    fn player_context() -> TestContext {
        TestContext {
            permissions: PermissionSet::default(),
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
        }
    }

    fn allow(permission: &str) -> PermissionEntry {
        PermissionEntry::allow(
            PermissionKey::parse(permission).expect("test permission key parses"),
        )
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

    #[test]
    fn command_execution_budget_rejects_after_limit() {
        let mut budget = CommandExecutionBudget::new(1);

        assert!(budget.consume().is_ok());
        assert!(budget.consume().is_err());
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

        super::queue_next_actions(
            &mut queue,
            [
                super::QueuedAction::Fallthrough(super::QueuedFrame::for_depth(1)),
                super::QueuedAction::Fallthrough(super::QueuedFrame::for_depth(2)),
            ],
        );

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
    fn aliases_reject_invalid_command_literals() {
        let minecraft = PermissionSegment::parse("minecraft").expect("namespace parses");
        let Err(error) = CommandRegistration::new(literal("root"), minecraft).alias("bad alias")
        else {
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
    fn parse_error_mapping_uses_vanilla_integer_bound_order() {
        let error = super::CommandParseError::new(
            CommandParseErrorKind::IntegerTooLow { value: 1, min: 5 },
            5,
        );

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
    }
}
