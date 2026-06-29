//! Handler for the "weather" command.
use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandNodeBuilder, CommandResult, ParsedArguments, argument, literal,
};
use crate::command::parsers::TimeParser;
use crate::command::CommandRegistrationSpec;
use steel_utils::translations;

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Handler for the "weather" command.
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
    literal("weather").then_all([
        weather_literal("rain", WeatherCommand::Rain),
        weather_literal("thunder", WeatherCommand::Thunder),
        weather_literal("clear", WeatherCommand::Clear),
    ])
}

fn weather_literal(name: &'static str, command: WeatherCommand) -> CommandNodeBuilder {
    literal(name)
        .executes(move |context: &mut CommandContext, _: &ParsedArguments| {
            execute_weather(context, command, command.random_duration())
        })
        .then(argument("duration", TimeParser).executes(
            move |context: &mut CommandContext, arguments: &ParsedArguments| {
                let duration = arguments
                    .get::<i32>("duration")
                    .map_err(super::invalid_parsed_argument)?;

                execute_weather(context, command, duration)
            },
        ))
}

#[derive(Clone, Copy)]
enum WeatherCommand {
    Clear,
    Rain,
    Thunder,
}

impl WeatherCommand {
    fn random_duration(self) -> i32 {
        match self {
            Self::Clear => rand::random_range(12_000..=180_000),
            Self::Rain => rand::random_range(12_000..=24_000),
            Self::Thunder => rand::random_range(3_600..=15_600),
        }
    }
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "Command executors use a shared fallible callback signature."
)]
fn execute_weather(
    context: &mut CommandContext,
    command: WeatherCommand,
    duration: i32,
) -> Result<CommandResult, CommandError> {
    let world = &context.world;
    let mut lock = world.level_data.write();
    let (clear_weather_time, weather_time, raining, thundering) = match command {
        WeatherCommand::Clear => (duration, 0, false, false),
        WeatherCommand::Rain => (0, duration, true, false),
        WeatherCommand::Thunder => (0, duration, true, true),
    };

    lock.set_clear_weather_time(clear_weather_time);
    lock.set_rain_time(weather_time);
    lock.set_thunder_time(weather_time);
    lock.set_raining(raining);
    lock.set_thundering(thundering);

    match command {
        WeatherCommand::Clear => {
            context
                .sender
                .send_message(&translations::COMMANDS_WEATHER_SET_CLEAR.msg().into());
        }
        WeatherCommand::Rain => {
            context
                .sender
                .send_message(&translations::COMMANDS_WEATHER_SET_RAIN.msg().into());
        }
        WeatherCommand::Thunder => {
            context
                .sender
                .send_message(&translations::COMMANDS_WEATHER_SET_THUNDER.msg().into());
        }
    }

    Ok(CommandResult::success())
}
