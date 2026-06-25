//! Handler for the "enchant" command.
//!
//! Vanilla targets any `LivingEntity`, but steel currently only supports players.
// TODO: Support all LivingEntity targets when the entity system supports it
use std::borrow::Cow;
use std::sync::Arc;

use steel_registry::enchantment::{Enchantment, EnchantmentRef};
use steel_utils::translations;
use text_components::translation::TranslatedMessage;
use text_components::{Modifier, TextComponent};

use crate::command::{CommandRegistration, CommandRegistrationError};
use crate::{
    command::{
        context::CommandContext,
        error::CommandError,
        graph::{
            CommandNodeBuilder, CommandResult, IntegerParser, ParsedArgumentError, ParsedArguments,
            argument, literal,
        },
        parsers::{EnchantmentParser, PlayerParser},
    },
    player::Player,
};

pub(crate) fn registration() -> Result<CommandRegistration, CommandRegistrationError> {
    CommandRegistration::minecraft(command())
}

/// Handler for the `/enchant` command.
#[must_use]
pub fn command() -> CommandNodeBuilder {
    literal("enchant").then(
        argument("targets", PlayerParser::multiple()).then(
            argument("enchantment", EnchantmentParser)
                .executes(enchant_default_level)
                .then(
                    argument("level", IntegerParser::bounded(Some(0), None))
                        .executes(enchant_with_level),
                ),
        ),
    )
}

fn enchant_default_level(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let enchantment = enchantment(arguments)?;

    enchant(&targets, enchantment, 1, context)?;

    Ok(CommandResult::success())
}

fn enchant_with_level(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let enchantment = enchantment(arguments)?;
    let level = level(arguments)?;

    enchant(&targets, enchantment, level, context)?;

    Ok(CommandResult::success())
}

fn enchant(
    targets: &[Arc<Player>],
    enchantment: EnchantmentRef,
    level: i32,
    ctx: &mut CommandContext,
) -> Result<(), CommandError> {
    if level > enchantment.max_level as i32 {
        return Err(CommandError::CommandFailed(Box::new(
            translations::COMMANDS_ENCHANT_FAILED_LEVEL
                .message([
                    TextComponent::from(level.to_string()),
                    TextComponent::from(enchantment.max_level.to_string()),
                ])
                .into(),
        )));
    }

    let mut success = 0u32;
    let enchantment_key = enchantment.key.clone();

    for target in targets {
        let mut inv = target.inventory.lock();
        let item = inv.get_selected_item();

        if item.is_empty() {
            if targets.len() == 1 {
                return Err(CommandError::CommandFailed(Box::new(
                    translations::COMMANDS_ENCHANT_FAILED_ITEMLESS
                        .message([TextComponent::from(target.gameprofile.name.clone())])
                        .into(),
                )));
            }
            continue;
        }

        if !enchantment.can_enchant(item.item)
            || !Enchantment::is_compatible_with_existing(enchantment, item)
        {
            if targets.len() == 1 {
                let item_name = item.item.key.to_string();
                return Err(CommandError::CommandFailed(Box::new(
                    translations::COMMANDS_ENCHANT_FAILED_INCOMPATIBLE
                        .message([TextComponent::from(item_name)])
                        .into(),
                )));
            }
            continue;
        }

        let item = inv.get_selected_item_mut();
        item.upgrade_enchantment(enchantment_key.clone(), level.max(0) as u32);
        success += 1;
    }

    if success == 0 {
        return Err(CommandError::CommandFailed(Box::new(
            translations::COMMANDS_ENCHANT_FAILED.msg().into(),
        )));
    }

    let enchantment_name = enchantment_display_name(enchantment, level);

    if let Some(target) = targets.first()
        && targets.len() == 1
    {
        ctx.sender.send_message(
            &translations::COMMANDS_ENCHANT_SUCCESS_SINGLE
                .message([
                    enchantment_name,
                    TextComponent::from(target.gameprofile.name.clone()),
                ])
                .into(),
        );
    } else {
        ctx.sender.send_message(
            &translations::COMMANDS_ENCHANT_SUCCESS_MULTIPLE
                .message([
                    enchantment_name,
                    TextComponent::from(targets.len().to_string()),
                ])
                .into(),
        );
    }

    Ok(())
}

fn targets(arguments: &ParsedArguments) -> Result<Vec<Arc<Player>>, CommandError> {
    arguments
        .get::<Vec<Arc<Player>>>("targets")
        .map_err(invalid_parsed_argument)
}

fn enchantment(arguments: &ParsedArguments) -> Result<EnchantmentRef, CommandError> {
    arguments
        .get::<EnchantmentRef>("enchantment")
        .map_err(invalid_parsed_argument)
}

fn level(arguments: &ParsedArguments) -> Result<i32, CommandError> {
    arguments
        .get::<i32>("level")
        .map_err(invalid_parsed_argument)
}

fn invalid_parsed_argument(error: ParsedArgumentError) -> CommandError {
    CommandError::InvalidConsumption(Some(format!("{error:?}")))
}

/// Builds a display name matching vanilla's `Enchantment.getFullname`:
/// translatable enchantment name + level suffix when level > 1 or `max_level` > 1.
fn enchantment_display_name(enchantment: EnchantmentRef, level: i32) -> TextComponent {
    let name_msg = TranslatedMessage {
        key: Cow::Owned(format!(
            "enchantment.{}.{}",
            enchantment.key.namespace, enchantment.key.path
        )),
        args: None,
        fallback: None,
    };
    let mut component = TextComponent::translated(name_msg);

    if level != 1 || enchantment.max_level != 1 {
        let level_msg = TranslatedMessage {
            key: Cow::Owned(format!("enchantment.level.{level}")),
            args: None,
            fallback: None,
        };
        component = component
            .add_child(TextComponent::plain(" "))
            .add_child(TextComponent::translated(level_msg));
    }

    component
}
