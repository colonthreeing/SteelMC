//! Handler for the "clear" command.
use std::sync::Arc;

use steel_registry::item_stack::ItemStack;
use steel_utils::translations;
use text_components::TextComponent;

use crate::command::CommandRegistrationSpec;
use crate::{
    command::{
        context::CommandContext,
        error::CommandError,
        graph::{
            CommandNodeBuilder, CommandResult, IntegerParser, ItemPredicateArgumentValue,
            ParsedArguments, argument, literal,
        },
        parsers::{ItemPredicateParser, PlayerParser},
        sender::CommandSender,
    },
    inventory::container::Container,
    player::Player,
};

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Handler for the "clear" command.
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
    literal("clear").executes(clear_self).then(
        argument("targets", PlayerParser::multiple())
            .executes(clear_targets)
            .then(
                argument("item", ItemPredicateParser)
                    .executes(clear_targets_with_item)
                    .then(
                        argument("maxCount", IntegerParser::bounded(Some(0), None))
                            .executes(clear_targets_with_max_amount),
                    ),
            ),
    )
}

fn clear_self(
    context: &mut CommandContext,
    _: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let player = context
        .sender
        .get_player()
        .ok_or(CommandError::InvalidRequirement)?;

    let count = { player.inventory.lock().clear_content() };

    clear_messages(
        &context.sender,
        count,
        1,
        Some(player.gameprofile.name.clone()),
        false,
    );

    Ok(CommandResult::success())
}

fn clear_targets(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;

    let count = targets
        .iter()
        .map(|player| player.inventory.lock().clear_content())
        .sum();

    clear_messages(
        &context.sender,
        count,
        targets.len(),
        targets.first().map(|it| it.gameprofile.name.clone()),
        false,
    );

    Ok(CommandResult::success())
}

fn clear_targets_with_item(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let predicate = item_predicate(arguments)?;
    let count = clear_targets_matching(&targets, &predicate, -1)?;

    clear_messages(
        &context.sender,
        count,
        targets.len(),
        targets.first().map(|it| it.gameprofile.name.clone()),
        false,
    );

    Ok(CommandResult::success())
}

fn clear_targets_with_max_amount(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let predicate = item_predicate(arguments)?;
    let max_amount = max_amount(arguments)?;
    let count = clear_targets_matching(&targets, &predicate, max_amount)?;

    clear_messages(
        &context.sender,
        count,
        targets.len(),
        targets.first().map(|it| it.gameprofile.name.clone()),
        max_amount == 0,
    );

    Ok(CommandResult::success())
}

fn clear_targets_matching(
    targets: &[Arc<Player>],
    predicate: &ItemPredicateArgumentValue,
    max_amount: i32,
) -> Result<i32, CommandError> {
    validate_item_predicate_targets(targets, predicate)?;

    let mut total = 0;
    for target in targets {
        total += clear_player_matching(target, predicate, max_amount)?;
    }
    Ok(total)
}

fn validate_item_predicate_targets(
    targets: &[Arc<Player>],
    predicate: &ItemPredicateArgumentValue,
) -> Result<(), CommandError> {
    for target in targets {
        let inventory = target.inventory.lock();
        for slot in 0..inventory.get_container_size() {
            let item = inventory.get_item(slot);
            if !item.is_empty() {
                predicate
                    .matches_stack(item)
                    .map_err(super::item_predicate_match_error)?;
            }
        }
    }
    Ok(())
}

fn clear_player_matching(
    target: &Player,
    predicate: &ItemPredicateArgumentValue,
    max_amount: i32,
) -> Result<i32, CommandError> {
    let mut inventory = target.inventory.lock();
    let mut matching_slots = Vec::new();
    for slot in 0..inventory.get_container_size() {
        let item = inventory.get_item(slot);
        if item.is_empty() {
            continue;
        }
        if predicate
            .matches_stack(item)
            .map_err(super::item_predicate_match_error)?
        {
            matching_slots.push(slot);
        }
    }

    let mut removed = 0;
    let mut current_amount = max_amount;
    let mut partial_change = false;
    for slot in matching_slots {
        if max_amount > 0 && current_amount == 0 {
            break;
        }

        let count = inventory.get_item(slot).count();
        if max_amount == 0 {
            removed += count;
        } else if max_amount < 0 {
            removed += count;
            inventory.set_item(slot, ItemStack::empty());
        } else {
            let amount_to_remove = current_amount.min(count);
            current_amount -= amount_to_remove;
            removed += amount_to_remove;
            inventory.get_item_mut(slot).shrink(amount_to_remove);
            partial_change = true;
        }
    }

    if partial_change {
        inventory.set_changed();
    }
    Ok(removed)
}

fn targets(arguments: &ParsedArguments) -> Result<Vec<Arc<Player>>, CommandError> {
    arguments
        .get::<Vec<Arc<Player>>>("targets")
        .map_err(super::invalid_parsed_argument)
}

fn item_predicate(arguments: &ParsedArguments) -> Result<ItemPredicateArgumentValue, CommandError> {
    arguments
        .get::<ItemPredicateArgumentValue>("item")
        .map_err(super::invalid_parsed_argument)
}

fn max_amount(arguments: &ParsedArguments) -> Result<i32, CommandError> {
    arguments
        .get::<i32>("maxCount")
        .map_err(super::invalid_parsed_argument)
}

fn clear_messages(
    sender: &CommandSender,
    count: i32,
    player_amount: usize,
    target_name: Option<String>,
    count_only: bool,
) {
    if count == 0
        && player_amount == 1
        && let Some(name) = target_name
    {
        sender.send_message(
            &translations::CLEAR_FAILED_SINGLE
                .message([TextComponent::from(name)])
                .into(),
        );
    } else if count == 0 {
        sender.send_message(
            &translations::CLEAR_FAILED_MULTIPLE
                .message([TextComponent::from(format!("{player_amount}"))])
                .into(),
        );
    } else if count_only
        && player_amount == 1
        && let Some(name) = target_name
    {
        sender.send_message(
            &translations::COMMANDS_CLEAR_TEST_SINGLE
                .message([
                    TextComponent::from(format!("{count}")),
                    TextComponent::from(name),
                ])
                .into(),
        );
    } else if count_only {
        sender.send_message(
            &translations::COMMANDS_CLEAR_TEST_MULTIPLE
                .message([
                    TextComponent::from(format!("{count}")),
                    TextComponent::from(format!("{player_amount}")),
                ])
                .into(),
        );
    } else if player_amount == 1
        && let Some(name) = target_name
    {
        sender.send_message(
            &translations::COMMANDS_CLEAR_SUCCESS_SINGLE
                .message([
                    TextComponent::from(format!("{count}")),
                    TextComponent::from(name),
                ])
                .into(),
        );
    } else {
        sender.send_message(
            &translations::COMMANDS_CLEAR_SUCCESS_MULTIPLE
                .message([
                    TextComponent::from(format!("{count}")),
                    TextComponent::from(format!("{player_amount}")),
                ])
                .into(),
        );
    }
}
