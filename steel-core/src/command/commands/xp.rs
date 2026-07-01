//! Experience Command

use std::sync::Arc;

use steel_utils::translations;
use text_components::TextComponent;

use crate::command::CommandRegistrationSpec;
use crate::{
    command::{
        context::CommandContext,
        error::CommandError,
        graph::{
            CommandNodeBuilder, CommandResult, IntegerParser, ParsedArguments, argument, literal,
        },
        parsers::{PlayerParser, resolve_required_player_targets},
    },
    player::Player,
};

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft()
    .permission_base("experience")
    .aliases(&["experience"]);

/// Handler for the "xp" command.
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
    literal("xp")
        .then(
            literal("query").then(
                argument("target", PlayerParser::one())
                    .then(literal("points").executes(query_points))
                    .then(literal("levels").executes(query_levels)),
            ),
        )
        .then(
            literal("set").then(
                argument("target", PlayerParser::multiple()).then(
                    argument("amount", IntegerParser::bounded(Some(0), None))
                        .executes(set_points)
                        .then(literal("points").executes(set_points))
                        .then(literal("levels").executes(set_levels)),
                ),
            ),
        )
        .then(
            literal("add").then(
                argument("target", PlayerParser::multiple()).then(
                    argument("amount", IntegerParser::new())
                        .executes(add_points)
                        .then(literal("points").executes(add_points))
                        .then(literal("levels").executes(add_levels)),
                ),
            ),
        )
        .then(
            literal("clear")
                .executes(clear_sender)
                .then(argument("target", PlayerParser::multiple()).executes(clear_targets)),
        )
}

fn query_points(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    query_experience(context, arguments, ExperienceType::Points)
}

fn query_levels(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    query_experience(context, arguments, ExperienceType::Levels)
}

fn query_experience(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
    xp_type: ExperienceType,
) -> Result<CommandResult, CommandError> {
    let player = single_player(arguments, context)?;
    let amount = {
        let experience = player.experience.lock();
        match xp_type {
            ExperienceType::Points => experience.points(),
            ExperienceType::Levels => experience.level(),
        }
    };
    let translation = match xp_type {
        ExperienceType::Points => &translations::COMMANDS_EXPERIENCE_QUERY_POINTS,
        ExperienceType::Levels => &translations::COMMANDS_EXPERIENCE_QUERY_LEVELS,
    };

    context.sender.send_message(
        &translation
            .message([
                TextComponent::from(player.gameprofile.name.clone()),
                TextComponent::from(amount.to_string()),
            ])
            .into(),
    );

    Ok(CommandResult::from_return_value(amount))
}

fn set_points(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    set_experience(
        players(arguments, context)?,
        amount(arguments)?,
        ExperienceType::Points,
        context,
    )
}

fn set_levels(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    set_experience(
        players(arguments, context)?,
        amount(arguments)?,
        ExperienceType::Levels,
        context,
    )
}

fn add_points(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let count = add_experience(
        players(arguments, context)?,
        amount(arguments)?,
        ExperienceType::Points,
        context,
    );
    Ok(CommandResult::from_usize_success_count(count))
}

fn add_levels(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let count = add_experience(
        players(arguments, context)?,
        amount(arguments)?,
        ExperienceType::Levels,
        context,
    );
    Ok(CommandResult::from_usize_success_count(count))
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "Command executors use a shared fallible callback signature."
)]
fn clear_sender(
    context: &mut CommandContext,
    _: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    if let Some(player) = context.sender.get_player() {
        player.experience.lock().set_total_points(0);
    }
    Ok(CommandResult::success())
}

fn clear_targets(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let players = players(arguments, context)?;
    let count = players.len();
    for player in players {
        player.experience.lock().set_total_points(0);
    }
    Ok(CommandResult::from_usize_success_count(count))
}

fn players(
    arguments: &ParsedArguments,
    context: &CommandContext,
) -> Result<Vec<Arc<Player>>, CommandError> {
    resolve_required_player_targets(arguments, "target", context)
}

fn single_player(
    arguments: &ParsedArguments,
    context: &CommandContext,
) -> Result<Arc<Player>, CommandError> {
    players(arguments, context)?
        .into_iter()
        .next()
        .ok_or_else(|| CommandError::InvalidConsumption(Some("target selector produced no players".to_owned())))
}

fn amount(arguments: &ParsedArguments) -> Result<i32, CommandError> {
    arguments
        .get::<i32>("amount")
        .map_err(super::invalid_parsed_argument)
}

#[derive(Clone, Copy)]
enum ExperienceType {
    Points,
    Levels,
}

fn set_experience(
    players: Vec<Arc<Player>>,
    amount: i32,
    xp_type: ExperienceType,
    ctx: &mut CommandContext,
) -> Result<CommandResult, CommandError> {
    for player in &players {
        let mut experience = player.experience.lock();
        match xp_type {
            ExperienceType::Points => experience
                .set_points(amount)
                .map_err(CommandError::failure)?,
            ExperienceType::Levels => experience.set_levels(amount),
        }
    }

    if let [player] = players.as_slice() {
        let translation = match xp_type {
            ExperienceType::Points => &translations::COMMANDS_EXPERIENCE_SET_POINTS_SUCCESS_SINGLE,
            ExperienceType::Levels => &translations::COMMANDS_EXPERIENCE_SET_LEVELS_SUCCESS_SINGLE,
        };

        ctx.sender.send_message(
            &translation
                .message([
                    TextComponent::from(amount.to_string()),
                    TextComponent::from(player.gameprofile.name.clone()),
                ])
                .into(),
        );
    } else {
        let translation = match xp_type {
            ExperienceType::Points => {
                &translations::COMMANDS_EXPERIENCE_SET_POINTS_SUCCESS_MULTIPLE
            }
            ExperienceType::Levels => {
                &translations::COMMANDS_EXPERIENCE_SET_LEVELS_SUCCESS_MULTIPLE
            }
        };

        ctx.sender.send_message(
            &translation
                .message([
                    TextComponent::from(amount.to_string()),
                    TextComponent::from(players.len().to_string()),
                ])
                .into(),
        );
    }

    Ok(CommandResult::from_usize_success_count(players.len()))
}

fn add_experience(
    players: Vec<Arc<Player>>,
    amount: i32,
    xp_type: ExperienceType,
    ctx: &mut CommandContext,
) -> usize {
    let count = players.len();
    for player in &players {
        let mut experience = player.experience.lock();
        match xp_type {
            ExperienceType::Points => experience.add_points(amount),
            ExperienceType::Levels => experience.add_levels(amount),
        }
    }

    if let [player] = players.as_slice() {
        let translation = match xp_type {
            ExperienceType::Points => &translations::COMMANDS_EXPERIENCE_ADD_POINTS_SUCCESS_SINGLE,
            ExperienceType::Levels => &translations::COMMANDS_EXPERIENCE_ADD_LEVELS_SUCCESS_SINGLE,
        };

        ctx.sender.send_message(
            &translation
                .message([
                    TextComponent::from(amount.to_string()),
                    TextComponent::from(player.gameprofile.name.clone()),
                ])
                .into(),
        );
    } else {
        let translation = match xp_type {
            ExperienceType::Points => {
                &translations::COMMANDS_EXPERIENCE_ADD_POINTS_SUCCESS_MULTIPLE
            }
            ExperienceType::Levels => {
                &translations::COMMANDS_EXPERIENCE_ADD_LEVELS_SUCCESS_MULTIPLE
            }
        };

        ctx.sender.send_message(
            &translation
                .message([
                    TextComponent::from(amount.to_string()),
                    TextComponent::from(players.len().to_string()),
                ])
                .into(),
            );
    }
    count
}
