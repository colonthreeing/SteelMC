//! Handler for the `time` command
use steel_protocol::packets::game::CSetTime;
use steel_registry::vanilla_game_rules::ADVANCE_TIME;
use steel_utils::translations;
use text_components::TextComponent;

use crate::command::CommandRegistrationSpec;
use crate::command::{
    context::CommandContext,
    error::CommandError,
    graph::{CommandNodeBuilder, CommandResult, ParsedArguments, argument, literal},
    parsers::TimeParser,
};
use crate::world::World;

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Handler for the `time` command
#[must_use]
pub fn command() -> CommandNodeBuilder {
    literal("time")
        .then(
            literal("query")
                .then(time_query_literal("day", TimeQuery::Day))
                .then(time_query_literal("daytime", TimeQuery::Daytime))
                .then(time_query_literal("gametime", TimeQuery::Gametime)),
        )
        .then(
            literal("set")
                .then(time_const_set_literal("day", 1000))
                .then(time_const_set_literal("midnight", 18_000))
                .then(time_const_set_literal("night", 13_000))
                .then(time_const_set_literal("noon", 6000))
                .then(argument("time", TimeParser).executes(set_parsed_time)),
        )
        .then(literal("add").then(argument("time", TimeParser).executes(add_time)))
}

fn time_query_literal(name: &'static str, query: TimeQuery) -> CommandNodeBuilder {
    literal(name).executes(move |context: &mut CommandContext, _: &ParsedArguments| {
        query_time(context, query)
    })
}

fn time_const_set_literal(name: &'static str, daytime: i64) -> CommandNodeBuilder {
    literal(name).executes(move |context: &mut CommandContext, _: &ParsedArguments| {
        set_const_time(context, daytime)
    })
}

#[derive(Clone, Copy)]
enum TimeQuery {
    Day,
    Daytime,
    Gametime,
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "Command executors use a shared fallible callback signature."
)]
fn query_time(
    context: &mut CommandContext,
    query: TimeQuery,
) -> Result<CommandResult, CommandError> {
    let number = {
        let lock = context.world.level_data.read();
        match query {
            TimeQuery::Day => lock.day(),
            TimeQuery::Daytime => lock.day_time(),
            TimeQuery::Gametime => lock.game_time(),
        }
    };
    context.sender.send_message(
        &translations::COMMANDS_TIME_QUERY
            .message([TextComponent::from(format!("{number}"))])
            .into(),
    );
    Ok(CommandResult::success())
}

#[derive(Clone, Copy)]
enum TimeOperation {
    Add,
    Set,
}

fn set_parsed_time(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let time = arguments
        .get::<i32>("time")
        .map_err(super::invalid_parsed_argument)?;
    apply_time(context, TimeOperation::Set, time)
}

fn add_time(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let time = arguments
        .get::<i32>("time")
        .map_err(super::invalid_parsed_argument)?;
    apply_time(context, TimeOperation::Add, time)
}

fn apply_time(
    context: &mut CommandContext,
    operation: TimeOperation,
    time: i32,
) -> Result<CommandResult, CommandError> {
    let mut day_time_option: Option<i64> = None;

    for world in context.server.worlds.values() {
        let (game_time, new_day_time) = {
            let mut lock = world.level_data.write();

            let game_time = lock.game_time();
            let new_day_time = match operation {
                TimeOperation::Add => (lock.day_time() + i64::from(time)) % 24_000,
                TimeOperation::Set => i64::from(time) % 24_000,
            };

            lock.set_day_time(new_day_time);
            (game_time, new_day_time)
        };

        let advance_time = advance_time(world.as_ref())?;

        day_time_option = Some(new_day_time);

        let rate = if advance_time { 1.0 } else { 0.0 };
        world.broadcast_to_all(CSetTime::new(game_time, new_day_time, 0.0, rate));
    }

    let Some(new_day_time) = day_time_option else {
        return Err(CommandError::failure("no world to update time on"));
    };

    send_time_set_message(context, new_day_time);

    Ok(CommandResult::success())
}

fn set_const_time(
    context: &mut CommandContext,
    daytime: i64,
) -> Result<CommandResult, CommandError> {
    for world in context.server.worlds.values() {
        let (game_time, new_day_time) = {
            let mut lock = world.level_data.write();

            let game_time = lock.game_time();
            let new_day_time = daytime;

            lock.set_day_time(new_day_time);
            (game_time, new_day_time)
        };

        let advance_time = advance_time(world.as_ref())?;

        let rate = if advance_time { 1.0 } else { 0.0 };
        world.broadcast_to_all(CSetTime::new(game_time, new_day_time, 0.0, rate));
    }

    send_time_set_message(context, daytime);

    Ok(CommandResult::success())
}

fn send_time_set_message(context: &CommandContext, daytime: i64) {
    context.sender.send_message(
        &translations::COMMANDS_TIME_SET
            .message([TextComponent::from(format!("{daytime}"))])
            .into(),
    );
}

fn advance_time(world: &World) -> Result<bool, CommandError> {
    world
        .get_game_rule(&ADVANCE_TIME)
        .as_bool()
        .ok_or_else(|| CommandError::failure("gamerule advance_time should always be a bool"))
}
