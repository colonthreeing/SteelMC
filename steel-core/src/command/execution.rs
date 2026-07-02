use std::{borrow::Cow, collections::VecDeque, sync::Arc};

use simdnbt::owned::NbtCompound;
use steel_registry::{
    game_rules::GameRuleValue,
    vanilla_game_rules::{MAX_COMMAND_FORKS, MAX_COMMAND_SEQUENCE_LENGTH},
};
use steel_utils::{Identifier, locks::SyncMutex};
use text_components::TextComponent;
use text_components::translation::TranslatedMessage;

use crate::command::context::{CommandCallbackResult, CommandContext, CommandResultCallback};
use crate::command::dispatcher::CommandDispatcher;
use crate::command::error::CommandError;
use crate::command::functions::{CommandFunction, InstantiatedCommandFunction};
use crate::command::graph::{CommandExecutionStep, CommandRedirectExecution, CommandResult};
use crate::command::requirement::CommandInputContext;
use crate::command::sender::CommandSender;

pub(super) const MAX_COMMAND_QUEUE_DEPTH: usize = 10_000_000;

pub(crate) struct CommandExecutionBudget {
    remaining: usize,
    limit: usize,
    pub(super) stopped: Option<CommandExecutionStop>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CommandExecutionStop {
    SequenceLimit,
    QueueOverflow,
}

impl CommandExecutionBudget {
    pub(super) fn for_context(context: &CommandContext) -> Self {
        Self::new(command_sequence_limit(context))
    }

    pub(super) fn new(limit: usize) -> Self {
        let limit = limit.max(1);
        Self {
            remaining: limit,
            limit,
            stopped: None,
        }
    }

    pub(crate) fn consume(&mut self) -> bool {
        if self.stopped.is_some() {
            return false;
        }

        if self.remaining == 0 {
            self.stop_due_to_sequence_limit();
            return false;
        }

        self.remaining -= 1;
        true
    }

    fn stop_due_to_sequence_limit(&mut self) {
        if self
            .stopped
            .replace(CommandExecutionStop::SequenceLimit)
            .is_none()
        {
            log::info!(
                "Command execution stopped due to limit (executed {} commands)",
                self.limit
            );
        }
    }

    fn stop_due_to_queue_overflow(&mut self) {
        if self
            .stopped
            .replace(CommandExecutionStop::QueueOverflow)
            .is_none()
        {
            log::error!(
                "Command execution stopped due to command queue overflow (max {})",
                MAX_COMMAND_QUEUE_DEPTH
            );
        }
    }

    fn is_stopped(&self) -> bool {
        self.stopped.is_some()
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

pub(super) fn fork_limit_reached(
    existing_stage_contexts: usize,
    new_stage_contexts: usize,
    limit: usize,
) -> bool {
    existing_stage_contexts.saturating_add(new_stage_contexts) >= limit
}

pub(super) fn command_fork_limit_error(limit: usize) -> CommandError {
    CommandError::failure(TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed("command.forkLimit"),
        fallback: None,
        args: Some(Box::new([TextComponent::from(limit.to_string())])),
    }))
}

impl CommandDispatcher {
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
        forked: bool,
    ) -> Result<CommandFunctionConditionResult, CommandError> {
        if functions.is_empty() {
            return Ok(CommandFunctionConditionResult::NoFunctions);
        }

        let function_context = function_execution_context(context);
        let prepared_functions = self.instantiate_functions_for_condition(functions, context);
        if !forked && let Some(failure) = &prepared_functions.failure {
            let error = command_function_instantiation_error(
                FunctionInstantiationFailureContext::ExecuteCondition,
                &failure.function_id,
                TextComponent::from(failure.reason.clone()),
            );
            context
                .sender
                .send_failure_feedback(error.into_feedback("execute if function"));
        }
        let mut queue = VecDeque::new();
        let function_frame = QueuedFrame::new(1, 1);
        for function in prepared_functions.functions {
            if !queue_action_back(
                &mut queue,
                QueuedAction::FunctionCall(QueuedFunctionCall::new_instantiated(
                    function,
                    function_context.clone(),
                    function_frame.clone(),
                    CommandResultCallback::empty(),
                    true,
                    true,
                )),
                budget,
            ) {
                return Ok(CommandFunctionConditionResult::NoFunctions);
            }
        }
        if !queue_action_back(
            &mut queue,
            QueuedAction::Fallthrough(function_frame),
            budget,
        ) {
            return Ok(CommandFunctionConditionResult::NoFunctions);
        }

        let mut frame_return = None;
        let Some(first) = next_queued_command(self, &mut queue, budget, &mut frame_return)? else {
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

    pub(super) fn instantiate_functions_for_condition(
        &self,
        functions: &[CommandFunction],
        context: &dyn CommandInputContext,
    ) -> PreparedFunctionConditionFunctions {
        let mut prepared = PreparedFunctionConditionFunctions {
            functions: Vec::with_capacity(functions.len()),
            failure: None,
        };
        for function in functions {
            match function.instantiate(None, self, context) {
                Ok(function) => prepared.functions.push(function),
                Err(error) => {
                    prepared.failure = Some(FunctionConditionInstantiationFailure {
                        function_id: function.id().clone(),
                        reason: error.to_string(),
                    });
                    break;
                }
            }
        }
        prepared
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
            if !budget.consume() {
                return Ok(dispatch_outcome(
                    total_success_count,
                    last_result,
                    completed_forked_context,
                    frame_return,
                ));
            }
            let active_command = active.command().to_owned();
            let active_forked = active.is_forked();
            let step = self.execute_command_step(
                &active_command,
                active.context_mut(),
                budget,
                active_forked,
            );
            let step = match step {
                Ok(step) => step,
                Err(_error) if active.continues_after_command_error() => {
                    let Some(next) =
                        next_queued_command(self, &mut queue, budget, &mut frame_return)?
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
                    let Some(next) =
                        next_queued_command(self, &mut queue, budget, &mut frame_return)?
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
                CommandExecutionStep::CallFunctions {
                    functions,
                    arguments,
                } => {
                    let active_frame = active.frame();
                    let return_parent_frame = active.is_returning();
                    let output_suppressed = active.context().is_output_suppressed();
                    let original_sender = active.context().sender.clone();
                    let original_callback = active.context().result_callback();
                    let function_context = function_execution_context(active.context());
                    let function_arguments = Arc::new(arguments);
                    let accumulates_results =
                        should_accumulate_function_results(functions.len(), &original_callback);
                    let mut actions = Vec::with_capacity(
                        functions
                            .len()
                            .saturating_add(usize::from(return_parent_frame))
                            .saturating_add(usize::from(accumulates_results)),
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
                                Arc::clone(&function_arguments),
                                function_context.clone(),
                                active_frame.clone(),
                                return_callback,
                                FunctionInstantiationFailureContext::FunctionCommand,
                                true,
                                true,
                            )));
                        }
                        actions.push(QueuedAction::Fallthrough(active_frame));
                    } else if accumulates_results {
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
                                Arc::clone(&function_arguments),
                                function_context.clone(),
                                active_frame.clone(),
                                return_callback,
                                FunctionInstantiationFailureContext::FunctionCommand,
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
                                Arc::clone(&function_arguments),
                                function_context.clone(),
                                active_frame.clone(),
                                return_callback,
                                FunctionInstantiationFailureContext::FunctionCommand,
                                false,
                                false,
                            )));
                        }
                    }
                    if !queue_next_actions(&mut queue, actions, budget) {
                        return Ok(dispatch_outcome(
                            total_success_count,
                            last_result,
                            completed_forked_context,
                            frame_return,
                        ));
                    }
                    let Some(next) =
                        next_queued_command(self, &mut queue, budget, &mut frame_return)?
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
                        if !self.collect_redirect_sibling_contexts(
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
                        )? {
                            return Ok(dispatch_outcome(
                                total_success_count,
                                last_result,
                                completed_forked_context,
                                frame_return,
                            ));
                        }
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
                    if !queue_next_actions(
                        &mut queue,
                        redirect_actions(
                            next_command,
                            contexts,
                            next_forked,
                            next_fork_stage,
                            next_returning,
                            next_frame,
                        ),
                        budget,
                    ) {
                        return Ok(dispatch_outcome(
                            total_success_count,
                            last_result,
                            completed_forked_context,
                            frame_return,
                        ));
                    }
                    let Some(next) =
                        next_queued_command(self, &mut queue, budget, &mut frame_return)?
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
        forked: bool,
    ) -> Result<CommandExecutionStep, CommandError> {
        match self.graph.parse(command, context) {
            Ok(parsed) => parsed
                .execute_step(context, budget, CommandRedirectExecution::new(forked))
                .map_err(|error| {
                    if parsed.invokes_result_callback_on_error() {
                        context.on_command_result(CommandCallbackResult {
                            success: false,
                            result: 0,
                        });
                    }
                    error
                }),
            Err(error) => {
                if Self::parse_error_invokes_result_callback(error.kind()) {
                    context.on_command_result(CommandCallbackResult {
                        success: false,
                        result: 0,
                    });
                }
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
    ) -> Result<bool, CommandError> {
        for mut sibling in drain_redirect_sibling_commands(queue, &batch) {
            if !budget.consume() {
                return Ok(false);
            }
            let step = self.execute_command_step(
                &sibling.command,
                &mut sibling.context,
                budget,
                sibling.forked,
            );
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

        Ok(true)
    }
}

fn function_execution_context(context: &CommandContext) -> CommandContext {
    // Vanilla suppresses output and grants a gamemaster permission level for
    // functions. Steel keeps the suppression/callback behavior but deliberately
    // preserves the captured caller permission authority; admins should get
    // command access through configured permission groups such as `op`.
    context
        .clone()
        .without_result_callbacks()
        .with_suppressed_output()
}

enum ActiveCommand<'a> {
    Borrowed {
        command: String,
        context: &'a mut CommandContext,
    },
    Owned(QueuedCommand),
}

pub(super) struct QueuedCommand {
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

pub(super) enum QueuedAction {
    Command(QueuedCommand),
    FunctionCall(QueuedFunctionCall),
    Fallthrough(QueuedFrame),
    Callback(QueuedCallbackAction),
}

impl QueuedAction {
    pub(super) fn frame_depth(&self) -> usize {
        match self {
            Self::Command(command) => command.frame.depth,
            Self::FunctionCall(call) => call.frame.depth,
            Self::Fallthrough(frame) => frame.depth,
            Self::Callback(callback) => callback.frame.depth,
        }
    }
}

pub(super) struct QueuedFunctionCall {
    function: QueuedFunction,
    context: CommandContext,
    frame: QueuedFrame,
    return_callback: CommandResultCallback,
    return_parent_frame: bool,
    propagates_return: bool,
}

enum QueuedFunction {
    Deferred {
        function: CommandFunction,
        arguments: Arc<Option<NbtCompound>>,
        instantiation_failure_context: FunctionInstantiationFailureContext,
    },
    Instantiated(InstantiatedCommandFunction),
}

impl QueuedFunctionCall {
    fn new(
        function: CommandFunction,
        arguments: Arc<Option<NbtCompound>>,
        context: CommandContext,
        frame: QueuedFrame,
        return_callback: CommandResultCallback,
        instantiation_failure_context: FunctionInstantiationFailureContext,
        return_parent_frame: bool,
        propagates_return: bool,
    ) -> Self {
        Self {
            function: QueuedFunction::Deferred {
                function,
                arguments,
                instantiation_failure_context,
            },
            context,
            frame,
            return_callback,
            return_parent_frame,
            propagates_return,
        }
    }

    fn new_instantiated(
        function: InstantiatedCommandFunction,
        context: CommandContext,
        frame: QueuedFrame,
        return_callback: CommandResultCallback,
        return_parent_frame: bool,
        propagates_return: bool,
    ) -> Self {
        Self {
            function: QueuedFunction::Instantiated(function),
            context,
            frame,
            return_callback,
            return_parent_frame,
            propagates_return,
        }
    }

    fn enqueue_commands(
        self,
        dispatcher: &CommandDispatcher,
        queue: &mut VecDeque<QueuedAction>,
        budget: &mut CommandExecutionBudget,
    ) -> Result<bool, CommandError> {
        let Self {
            function,
            context,
            frame,
            return_callback,
            return_parent_frame,
            propagates_return,
        } = self;
        let instantiated = match function {
            QueuedFunction::Deferred {
                function,
                arguments,
                instantiation_failure_context,
            } => function
                .instantiate(arguments.as_ref().as_ref(), dispatcher, &context)
                .map_err(|error| {
                    command_function_instantiation_error(
                        instantiation_failure_context,
                        function.id(),
                        TextComponent::from(error.to_string()),
                    )
                })?,
            QueuedFunction::Instantiated(function) => function,
        };
        let child_frame = if return_parent_frame {
            QueuedFrame::with_return_callback(
                frame.depth.saturating_add(1),
                frame.return_discard_depth,
                return_callback,
                propagates_return,
            )
        } else {
            QueuedFrame::with_return_callback(
                frame.depth.saturating_add(1),
                frame.depth.saturating_add(1),
                return_callback,
                propagates_return,
            )
        };
        for command in instantiated.commands().iter().rev() {
            if !queue_action_front(
                queue,
                QueuedAction::Command(QueuedCommand::new(
                    command.clone(),
                    context.clone(),
                    false,
                    0,
                    false,
                    child_frame.clone(),
                )),
                budget,
            ) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

#[derive(Clone, Copy)]
pub(super) enum FunctionInstantiationFailureContext {
    FunctionCommand,
    ExecuteCondition,
}

impl FunctionInstantiationFailureContext {
    const fn translation_key(self) -> &'static str {
        match self {
            Self::FunctionCommand => "commands.function.instantiationFailure",
            Self::ExecuteCondition => "commands.execute.function.instantiationFailure",
        }
    }
}

pub(super) struct QueuedCallbackAction {
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
pub(super) struct QueuedFrame {
    pub(super) depth: usize,
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

    pub(super) fn with_return_callback(
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

    pub(super) fn for_depth(depth: usize) -> Self {
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

pub(super) struct PreparedFunctionConditionFunctions {
    pub(super) functions: Vec<InstantiatedCommandFunction>,
    pub(super) failure: Option<FunctionConditionInstantiationFailure>,
}

pub(super) struct FunctionConditionInstantiationFailure {
    pub(super) function_id: Identifier,
    pub(super) reason: String,
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

pub(super) fn should_accumulate_function_results(
    function_count: usize,
    callback: &CommandResultCallback,
) -> bool {
    function_count > 1 && !callback.is_empty()
}

pub(super) fn decorated_function_callback(
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

pub(super) fn function_result_message(function_id: &Identifier, result: i32) -> TextComponent {
    TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed("commands.function.result"),
        fallback: None,
        args: Some(Box::new([
            TextComponent::from(function_id.to_string()),
            TextComponent::from(result.to_string()),
        ])),
    })
}

pub(super) fn command_function_instantiation_error(
    context: FunctionInstantiationFailureContext,
    function_id: &Identifier,
    reason: TextComponent,
) -> CommandError {
    CommandError::failure(TextComponent::translated(TranslatedMessage {
        key: Cow::Borrowed(context.translation_key()),
        fallback: None,
        args: Some(Box::new([
            TextComponent::from(function_id.to_string()),
            reason,
        ])),
    }))
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

pub(super) struct RedirectSiblingBatch<'a> {
    pub(super) active_command: &'a str,
    pub(super) active_fork_stage: usize,
    pub(super) active_returning: bool,
    pub(super) active_frame: &'a QueuedFrame,
    pub(super) next_command: &'a str,
    pub(super) forked: bool,
    pub(super) returns: bool,
}

pub(super) fn queue_next_actions(
    queue: &mut VecDeque<QueuedAction>,
    actions: impl IntoIterator<Item = QueuedAction>,
    budget: &mut CommandExecutionBudget,
) -> bool {
    let actions = actions.into_iter().collect::<Vec<_>>();
    for action in actions.into_iter().rev() {
        if !queue_action_front(queue, action, budget) {
            return false;
        }
    }
    true
}

fn queue_action_front(
    queue: &mut VecDeque<QueuedAction>,
    action: QueuedAction,
    budget: &mut CommandExecutionBudget,
) -> bool {
    if queue_would_overflow(queue, budget) {
        return false;
    }
    queue.push_front(action);
    true
}

fn queue_action_back(
    queue: &mut VecDeque<QueuedAction>,
    action: QueuedAction,
    budget: &mut CommandExecutionBudget,
) -> bool {
    if queue_would_overflow(queue, budget) {
        return false;
    }
    queue.push_back(action);
    true
}

fn queue_would_overflow(
    queue: &mut VecDeque<QueuedAction>,
    budget: &mut CommandExecutionBudget,
) -> bool {
    if budget.is_stopped() {
        return true;
    }
    if !queue_len_exceeds_max(queue.len()) {
        return false;
    }
    budget.stop_due_to_queue_overflow();
    queue.clear();
    true
}

pub(super) const fn queue_len_exceeds_max(len: usize) -> bool {
    len > MAX_COMMAND_QUEUE_DEPTH
}

pub(super) fn redirect_actions(
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

pub(super) fn redirect_sibling_matches(
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

pub(super) fn return_from_frame(
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
    dispatcher: &CommandDispatcher,
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
                if !budget.consume() {
                    return Ok(None);
                }
                if !call.enqueue_commands(dispatcher, queue, budget)? {
                    return Ok(None);
                }
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

pub(super) fn discard_command_frame(queue: &mut VecDeque<QueuedAction>, frame_depth: usize) {
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

pub(super) fn execution_success_count(result: CommandResult, forked: bool) -> i32 {
    if forked { 1 } else { result.return_value() }
}

pub(super) fn command_callback_result(result: CommandResult) -> CommandCallbackResult {
    CommandCallbackResult {
        success: result.callback_success(),
        result: result.return_value(),
    }
}

pub(super) fn dispatch_result(
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
