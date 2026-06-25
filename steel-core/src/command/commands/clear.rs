//! Handler for the "clear" command.
use std::sync::Arc;

use steel_registry::{item_stack::ItemStack, items::ItemRef};
use steel_utils::translations;
use text_components::TextComponent;

use crate::command::{CommandRegistration, CommandRegistrationError};
use crate::{
    command::{
        context::CommandContext,
        error::CommandError,
        graph::{
            CommandNodeBuilder, CommandResult, IntegerParser, ParsedArguments, argument, literal,
        },
        parsers::{ItemParser, PlayerParser},
        sender::CommandSender,
    },
    inventory::container::Container,
    player::Player,
};

pub(crate) fn registration() -> Result<CommandRegistration, CommandRegistrationError> {
    CommandRegistration::minecraft(command())
}

/// Handler for the "clear" command.
#[must_use]
pub fn command() -> CommandNodeBuilder {
    literal("clear").executes(clear_self).then(
        argument("targets", PlayerParser::multiple())
            .executes(clear_targets)
            .then(
                argument("item", ItemParser)
                    .executes(clear_targets_with_item) // FIXME: item predicate instead
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
    let item = item(arguments)?;
    let mut filter = |item_stack: &mut ItemStack| item_stack.is(item);

    let count: i32 = targets
        .iter()
        .map(|it| it.inventory.lock().clear_content_matching(&mut filter))
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

fn clear_targets_with_max_amount(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let item = item(arguments)?;
    let max_amount = max_amount(arguments)?;

    let count: i32 = targets
        .iter()
        .map(|it| {
            let mut current_amount = max_amount;
            let mut inventory = it.inventory.lock();
            let mut removed = 0;
            for i in 0..inventory.get_container_size() {
                if max_amount > 0 && current_amount == 0 {
                    break;
                }
                let current_item = inventory.get_item_mut(i);
                if current_item.is_empty() || !current_item.is(item) {
                    continue;
                }
                if max_amount == 0 {
                    removed += current_item.count();
                } else {
                    let amount_to_remove = current_amount.min(current_item.count());
                    current_amount -= amount_to_remove;
                    removed += amount_to_remove;
                    current_item.shrink(amount_to_remove);
                }
            }
            if max_amount > 0 && removed > 0 {
                inventory.set_changed();
            }
            removed
        })
        .sum();

    clear_messages(
        &context.sender,
        count,
        targets.len(),
        targets.first().map(|it| it.gameprofile.name.clone()),
        max_amount == 0,
    );

    Ok(CommandResult::success())
}

fn targets(arguments: &ParsedArguments) -> Result<Vec<Arc<Player>>, CommandError> {
    arguments
        .get::<Vec<Arc<Player>>>("targets")
        .map_err(super::invalid_parsed_argument)
}

fn item(arguments: &ParsedArguments) -> Result<ItemRef, CommandError> {
    arguments
        .get::<ItemRef>("item")
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
