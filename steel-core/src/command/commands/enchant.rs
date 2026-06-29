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

use crate::command::CommandRegistrationSpec;
use crate::{
    command::{
        context::CommandContext,
        error::CommandError,
        graph::{
            CommandNodeBuilder, CommandResult, IntegerParser, ParsedArguments, argument, literal,
        },
        parsers::{EnchantmentParser, PlayerParser},
    },
    player::Player,
};

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Handler for the `/enchant` command.
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
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

    Ok(CommandResult::from_success_count(enchant(
        &targets,
        enchantment,
        1,
        context,
    )?))
}

fn enchant_with_level(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let targets = targets(arguments)?;
    let enchantment = enchantment(arguments)?;
    let level = level(arguments)?;

    Ok(CommandResult::from_success_count(enchant(
        &targets,
        enchantment,
        level,
        context,
    )?))
}

fn enchant(
    targets: &[Arc<Player>],
    enchantment: EnchantmentRef,
    level: i32,
    ctx: &mut CommandContext,
) -> Result<i32, CommandError> {
    if level > enchantment.max_level as i32 {
        return Err(CommandError::failure(
            translations::COMMANDS_ENCHANT_FAILED_LEVEL.message([
                TextComponent::from(level.to_string()),
                TextComponent::from(enchantment.max_level.to_string()),
            ]),
        ));
    }

    let mut success = 0;
    let enchantment_key = enchantment.key.clone();

    for target in targets {
        let mut inv = target.inventory.lock();
        let item = inv.get_selected_item();

        if item.is_empty() {
            if targets.len() == 1 {
                return Err(CommandError::failure(
                    translations::COMMANDS_ENCHANT_FAILED_ITEMLESS
                        .message([TextComponent::from(target.gameprofile.name.clone())]),
                ));
            }
            continue;
        }

        if !enchantment.can_enchant(item.item)
            || !Enchantment::is_compatible_with_existing(enchantment, item)
        {
            if targets.len() == 1 {
                let item_name = item.item.key.to_string();
                return Err(CommandError::failure(
                    translations::COMMANDS_ENCHANT_FAILED_INCOMPATIBLE
                        .message([TextComponent::from(item_name)]),
                ));
            }
            continue;
        }

        let item = inv.get_selected_item_mut();
        item.upgrade_enchantment(enchantment_key.clone(), level.max(0) as u32);
        success += 1;
    }

    if success == 0 {
        return Err(CommandError::failure(
            translations::COMMANDS_ENCHANT_FAILED.msg(),
        ));
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

    Ok(success)
}

fn targets(arguments: &ParsedArguments) -> Result<Vec<Arc<Player>>, CommandError> {
    arguments
        .get::<Vec<Arc<Player>>>("targets")
        .map_err(super::invalid_parsed_argument)
}

fn enchantment(arguments: &ParsedArguments) -> Result<EnchantmentRef, CommandError> {
    arguments
        .get::<EnchantmentRef>("enchantment")
        .map_err(super::invalid_parsed_argument)
}

fn level(arguments: &ParsedArguments) -> Result<i32, CommandError> {
    arguments
        .get::<i32>("level")
        .map_err(super::invalid_parsed_argument)
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
