//! Handler for the "tick" command.
use steel_utils::translations;
use text_components::TextComponent;

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandNodeBuilder, CommandResult, FloatParser, ParsedArguments, argument, literal,
};
use crate::command::parsers::TimeParser;
use crate::command::CommandRegistrationSpec;

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Handler for the "tick" command.
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
    literal("tick")
        .then(literal("query").executes(query_tick))
        .then(
            literal("rate").requires_subcommand_permission().then(
                argument("rate", FloatParser::bounded(Some(1.0), Some(10_000.0)))
                    .executes(set_tick_rate),
            ),
        )
        .then(
            literal("freeze")
                .requires_subcommand_permission()
                .executes(freeze_tick),
        )
        .then(
            literal("unfreeze")
                .requires_subcommand_permission()
                .executes(unfreeze_tick),
        )
        .then(
            literal("step")
                .requires_subcommand_permission()
                .executes(step_default)
                .then(literal("stop").executes(stop_step))
                .then(argument("time", TimeParser).executes(step_ticks)),
        )
        .then(
            literal("sprint")
                .requires_subcommand_permission()
                .then(literal("stop").executes(stop_sprint))
                .then(argument("time", TimeParser).executes(sprint_ticks)),
        )
}

/// Converts nanoseconds to a formatted millisecond string.
fn nanos_to_ms_string(nanos: u64) -> String {
    format!("{:.1}", nanos as f64 / 1_000_000.0)
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "Command executors use a shared fallible callback signature."
)]
fn query_tick(
    context: &mut CommandContext,
    _: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let tick_manager = context.server.tick_rate_manager.read();

    let tick_rate = tick_manager.tick_rate();
    let busy_time_nanos = tick_manager.get_average_tick_time_nanos();
    let busy_time = nanos_to_ms_string(busy_time_nanos);
    let tick_rate_string = format!("{tick_rate:.1}");

    // Send status and rate info based on current state
    if tick_manager.is_sprinting() {
        context
            .sender
            .send_message(&translations::COMMANDS_TICK_STATUS_SPRINTING.msg().into());
        context.sender.send_message(
            &translations::COMMANDS_TICK_QUERY_RATE_SPRINTING
                .message([
                    TextComponent::from(tick_rate_string),
                    TextComponent::from(busy_time),
                ])
                .into(),
        );
    } else {
        // Determine status
        if tick_manager.is_frozen() {
            context
                .sender
                .send_message(&translations::COMMANDS_TICK_STATUS_FROZEN.msg().into());
        } else if tick_manager.nanoseconds_per_tick < busy_time_nanos {
            context
                .sender
                .send_message(&translations::COMMANDS_TICK_STATUS_LAGGING.msg().into());
        } else {
            context
                .sender
                .send_message(&translations::COMMANDS_TICK_STATUS_RUNNING.msg().into());
        }

        let target_mspt = nanos_to_ms_string(tick_manager.nanoseconds_per_tick);
        context.sender.send_message(
            &translations::COMMANDS_TICK_QUERY_RATE_RUNNING
                .message([
                    TextComponent::from(tick_rate_string),
                    TextComponent::from(busy_time),
                    TextComponent::from(target_mspt),
                ])
                .into(),
        );
    }

    // Get percentiles (vanilla sorts and calculates from the raw samples)
    let mut samples = tick_manager.get_tick_times_nanos();
    let sample_count = tick_manager.get_sample_count();
    drop(tick_manager);

    samples[..sample_count].sort_unstable();

    let p50 = if sample_count > 0 {
        nanos_to_ms_string(samples[sample_count / 2])
    } else {
        "0.0".to_string()
    };
    let p95 = if sample_count > 0 {
        nanos_to_ms_string(samples[(sample_count as f64 * 0.95) as usize])
    } else {
        "0.0".to_string()
    };
    let p99 = if sample_count > 0 {
        nanos_to_ms_string(samples[(sample_count as f64 * 0.99) as usize])
    } else {
        "0.0".to_string()
    };

    context.sender.send_message(
        &translations::COMMANDS_TICK_QUERY_PERCENTILES
            .message([
                TextComponent::from(p50),
                TextComponent::from(p95),
                TextComponent::from(p99),
                TextComponent::from(format!("{sample_count}")),
            ])
            .into(),
    );

    Ok(CommandResult::success())
}

fn set_tick_rate(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let rate = arguments
        .get::<f32>("rate")
        .map_err(super::invalid_parsed_argument)?;

    context.server.broadcast_ticking_state();
    context.server.tick_rate_manager.write().set_tick_rate(rate);

    let rate_string = format!("{rate:.1}");
    context.sender.send_message(
        &translations::COMMANDS_TICK_RATE_SUCCESS
            .message([TextComponent::from(rate_string)])
            .into(),
    );

    Ok(CommandResult::success())
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "Command executors use a shared fallible callback signature."
)]
fn freeze_tick(
    context: &mut CommandContext,
    _: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let mut tick_manager = context.server.tick_rate_manager.write();

    // Stop sprinting if active (vanilla behavior)
    if tick_manager.is_sprinting() {
        tick_manager.stop_sprinting();
    }

    // Stop stepping if active (vanilla behavior)
    if tick_manager.is_stepping_forward() {
        tick_manager.stop_stepping();
    }

    tick_manager.set_frozen(true);
    drop(tick_manager);

    context.server.broadcast_ticking_state();

    context
        .sender
        .send_message(&translations::COMMANDS_TICK_STATUS_FROZEN.msg().into());

    Ok(CommandResult::success())
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "Command executors use a shared fallible callback signature."
)]
fn unfreeze_tick(
    context: &mut CommandContext,
    _: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    context.server.tick_rate_manager.write().set_frozen(false);
    context.server.broadcast_ticking_state();

    context
        .sender
        .send_message(&translations::COMMANDS_TICK_STATUS_RUNNING.msg().into());

    Ok(CommandResult::success())
}

fn step_default(
    context: &mut CommandContext,
    _: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    step_impl(1, context)
}

fn step_ticks(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let ticks = arguments
        .get::<i32>("time")
        .map_err(super::invalid_parsed_argument)?;
    step_impl(ticks, context)
}

fn step_impl(ticks: i32, context: &mut CommandContext) -> Result<CommandResult, CommandError> {
    let success = context
        .server
        .tick_rate_manager
        .write()
        .step_game_if_paused(ticks);

    if success {
        context.server.broadcast_ticking_step();
        context.sender.send_message(
            &translations::COMMANDS_TICK_STEP_SUCCESS
                .message([TextComponent::from(format!("{ticks}"))])
                .into(),
        );
        Ok(CommandResult::success())
    } else {
        Err(CommandError::failure(
            translations::COMMANDS_TICK_STEP_FAIL.msg(),
        ))
    }
}

fn stop_step(
    context: &mut CommandContext,
    _: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let stopped = context.server.tick_rate_manager.write().stop_stepping();

    if stopped {
        context.server.broadcast_ticking_step();
        context
            .sender
            .send_message(&translations::COMMANDS_TICK_STEP_STOP_SUCCESS.msg().into());
        Ok(CommandResult::success())
    } else {
        Err(CommandError::failure(
            translations::COMMANDS_TICK_STEP_STOP_FAIL.msg(),
        ))
    }
}

fn sprint_ticks(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let ticks = arguments
        .get::<i32>("time")
        .map_err(super::invalid_parsed_argument)?;

    let interrupted = context
        .server
        .tick_rate_manager
        .write()
        .request_game_to_sprint(ticks);

    // Broadcast state change (unfrozen during sprint)
    context.server.broadcast_ticking_state();

    if interrupted {
        context
            .sender
            .send_message(&translations::COMMANDS_TICK_SPRINT_STOP_SUCCESS.msg().into());
    }

    context
        .sender
        .send_message(&translations::COMMANDS_TICK_STATUS_SPRINTING.msg().into());

    Ok(CommandResult::success())
}

fn stop_sprint(
    context: &mut CommandContext,
    _: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let report = context.server.tick_rate_manager.write().stop_sprinting();

    if let Some(report) = report {
        // Broadcast state change (restored previous frozen state)
        context.server.broadcast_ticking_state();

        // Send sprint report
        context.sender.send_message(
            &translations::COMMANDS_TICK_SPRINT_REPORT
                .message([
                    TextComponent::from(format!("{}", report.ticks_per_second)),
                    TextComponent::from(format!("{:.2}", report.ms_per_tick)),
                ])
                .into(),
        );
        Ok(CommandResult::success())
    } else {
        Err(CommandError::failure(
            translations::COMMANDS_TICK_SPRINT_STOP_FAIL.msg(),
        ))
    }
}
