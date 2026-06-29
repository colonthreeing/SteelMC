//! /// Handler for the "give" command.
use std::sync::Arc;

use steel_registry::{data_components::vanilla_components, item_stack::ItemStack, items::ItemRef};
use steel_utils::translations;
use text_components::{Modifier, TextComponent, interactivity::HoverEvent};

use crate::command::CommandRegistrationSpec;
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

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Handler for the "give" command.
#[must_use]
pub fn command() -> CommandNodeBuilder {
    literal("give").then(
        argument("targets", PlayerParser::multiple()).then(
            argument("item", ItemParser) // FIXME: should be item predicate instead to also handle tags and components
                .executes(give_default_count)
                .then(
                    argument("count", IntegerParser::bounded(Some(1), None))
                        .executes(give_with_count),
                ),
        ),
    )
}

fn give_default_count(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let item = item(arguments)?;

    give(&targets, item, 1, &context.sender);

    Ok(CommandResult::success())
}

fn give_with_count(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let item = item(arguments)?;
    let count = count(arguments)?;

    give(&targets, item, count, &context.sender);

    Ok(CommandResult::success())
}

fn give(targets: &[Arc<Player>], item: ItemRef, count: i32, sender: &CommandSender) {
    let max_stack_size = item
        .components
        .get(vanilla_components::MAX_STACK_SIZE)
        .unwrap_or(1);

    if count > max_stack_size * 100 {
        sender.send_message(
            &translations::COMMANDS_GIVE_FAILED_TOOMANYITEMS
                .message([
                    TextComponent::from(format!("{}", max_stack_size * 100)),
                    TextComponent::from(format!("[{}]", item.key.path)).hover_event(
                        // FIXME: display name
                        HoverEvent::show_item(item.key.path.clone(), None, None::<&str>),
                    ),
                ])
                .into(),
        );
        return;
    }

    let stack = ItemStack::new(item);

    for target in targets {
        let mut remaining = count;

        while remaining > 0 {
            let stack_size = max_stack_size.min(remaining);
            remaining -= stack_size;
            let mut copy = stack.copy_with_count(stack_size);
            let added = target.inventory.lock().add(&mut copy);

            if !added || !copy.is_empty() {
                target.drop_item(copy, false, false);
            }
        }
    }

    if let Some(target) = targets.first()
        && targets.len() == 1
    {
        sender.send_message(
            &translations::COMMANDS_GIVE_SUCCESS_SINGLE
                .message([
                    TextComponent::from(format!("{count}")),
                    TextComponent::from(format!("[{}]", item.key.path)).hover_event(
                        // FIXME: display name
                        HoverEvent::show_item(item.key.path.clone(), None, None::<&str>),
                    ),
                    TextComponent::from(target.gameprofile.name.clone()),
                ])
                .into(),
        );
    } else {
        sender.send_message(
            &translations::COMMANDS_GIVE_SUCCESS_MULTIPLE
                .message([
                    TextComponent::from(format!("{count}")),
                    TextComponent::from(format!("[{}]", item.key.path)).hover_event(
                        // FIXME: display name
                        HoverEvent::show_item(item.key.path.clone(), None, None::<&str>),
                    ),
                    TextComponent::from(targets.len().to_string()),
                ])
                .into(),
        );
    }
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

fn count(arguments: &ParsedArguments) -> Result<i32, CommandError> {
    arguments
        .get::<i32>("count")
        .map_err(super::invalid_parsed_argument)
}
