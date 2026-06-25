//! Command implementations.

use crate::command::{CommandRegistration, CommandRegistrationError};

pub mod clear;
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
pub mod seed;
pub mod setworldspawn;
pub mod steel;
pub mod stop;
pub mod summon;
pub mod tellraw;
pub mod tick;
pub mod time;
pub mod tp;
pub mod weather;
pub mod xp;

type RegistrationFactory = fn() -> Result<CommandRegistration, CommandRegistrationError>;

const BUILT_IN_COMMANDS: &[RegistrationFactory] = &[
    clear::registration,
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
    xp::registration,
];

pub(super) fn registrations() -> Result<Vec<CommandRegistration>, CommandRegistrationError> {
    BUILT_IN_COMMANDS
        .iter()
        .map(|registration| registration())
        .collect()
}
