//! Handler for the "seed" command.
use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{CommandNodeBuilder, CommandResult, ParsedArguments, literal};
use steel_utils::translations;
use text_components::format::Color;
use text_components::interactivity::{ClickEvent, HoverEvent};
use text_components::{Modifier, TextComponent};

/// Handler for the "seed" command.
#[must_use]
pub fn command() -> CommandNodeBuilder {
    literal("seed").executes(send_seed)
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "Command executors use a shared fallible callback signature."
)]
fn send_seed(
    context: &mut CommandContext,
    _: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let seed = context.world.seed().to_string();
    context.sender.send_message(
        &translations::COMMANDS_SEED_SUCCESS
            .message([TextComponent::from(seed.clone())
                .color(Color::Green)
                .hover_event(HoverEvent::show_text(&translations::CHAT_COPY_CLICK))
                .click_event(ClickEvent::CopyToClipboard { value: seed.into() })])
            .component(),
    );
    Ok(CommandResult::success())
}
