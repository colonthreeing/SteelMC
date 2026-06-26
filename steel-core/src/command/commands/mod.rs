//! Command implementations.

use crate::command::{
    CommandRegistration, CommandRegistrationError, error::CommandError, graph::ParsedArgumentError,
};

pub mod clear;
pub mod deop;
pub mod difficulty;
pub mod domain;
pub mod enchant;
pub mod execute;
pub mod fly;
pub mod gamemode;
pub mod gamerule;
pub mod give;
pub mod kill;
pub mod list;
pub mod locate;
pub mod op;
mod permission_targets;
pub mod seed;
pub mod setworldspawn;
pub mod steel;
pub mod steelperms;
pub mod stop;
pub mod summon;
pub mod tellraw;
pub mod tick;
pub mod time;
pub mod tp;
pub mod weather;
pub mod xp;

type RegistrationFactory = fn() -> Result<CommandRegistration, CommandRegistrationError>;

fn invalid_parsed_argument(error: ParsedArgumentError) -> CommandError {
    CommandError::InvalidConsumption(Some(format!("{error:?}")))
}

const BUILT_IN_COMMANDS: &[RegistrationFactory] = &[
    clear::registration,
    deop::registration,
    domain::registration,
    enchant::registration,
    execute::registration,
    fly::registration,
    gamemode::registration,
    gamerule::registration,
    kill::registration,
    list::registration,
    locate::registration,
    give::registration,
    op::registration,
    seed::registration,
    setworldspawn::registration,
    stop::registration,
    summon::registration,
    tellraw::registration,
    tick::registration,
    time::registration,
    tp::registration,
    weather::registration,
    difficulty::registration,
    steel::registration,
    steelperms::registration,
    xp::registration,
];

pub(super) fn registrations() -> Result<Vec<CommandRegistration>, CommandRegistrationError> {
    BUILT_IN_COMMANDS
        .iter()
        .map(|registration| registration())
        .collect()
}
