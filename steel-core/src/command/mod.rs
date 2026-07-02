//! Command parsing, execution, and sender handling.
pub mod commands;
pub mod context;
mod dispatcher;
pub mod error;
mod execution;
mod executor;
pub mod functions;
pub mod graph;
mod loot;
pub mod parsers;
pub mod reader;
mod registration;
pub mod requirement;
pub mod sender;
pub mod storage;
pub(crate) mod suggestions;

pub use dispatcher::CommandDispatcher;
pub use registration::CommandRegistrationError;

pub(crate) use execution::{CommandExecutionBudget, CommandFunctionConditionResult};
pub(crate) use executor::CommandQueue;
pub(crate) use registration::{
    CommandRegistration, CommandRegistrationSpec, entity_selector_advanced_permission_expr,
    entity_selector_permission_expr, minecraft_command_permission_key,
};

#[cfg(test)]
pub(crate) use registration::{
    ENTITY_SELECTOR_ADVANCED_PERMISSION_KEY, ENTITY_SELECTOR_PERMISSION_KEY,
};

#[cfg(test)]
use context::CommandCallbackResult;
#[cfg(test)]
use execution::{
    CommandExecutionStop, FunctionInstantiationFailureContext, MAX_COMMAND_QUEUE_DEPTH,
    QueuedAction, QueuedFrame, RedirectSiblingBatch, command_callback_result,
    command_fork_limit_error, command_function_instantiation_error, decorated_function_callback,
    discard_command_frame, dispatch_result, execution_success_count, fork_limit_reached,
    function_result_message, queue_len_exceeds_max, queue_next_actions, redirect_actions,
    redirect_sibling_matches, return_from_frame, should_accumulate_function_results,
};
#[cfg(test)]
use graph::CommandParseError;
#[cfg(test)]
use registration::command_permission_key;
#[cfg(test)]
use steel_protocol::packets::game::CCommands;

#[cfg(test)]
mod tests;
