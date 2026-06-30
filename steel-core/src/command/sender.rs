//! Module defining the sender of a command.
use std::{fmt, sync::Arc};
use text_components::{Modifier, TextComponent, format::Color};

use crate::command::error::CommandErrorFeedback;
use crate::player::Player;

/// The sender of a command.
#[derive(Clone)]
pub enum CommandSender {
    /// The command was sent by a player via the chat.
    Player(Arc<Player>),
    /// The command was sent via the server's console.
    Console,
    /// The command was sent via Rcon.
    Rcon,
    /// The command keeps the same source identity, but drops command output.
    SuppressedOutput(Arc<CommandSender>),
}

impl CommandSender {
    /// Returns the player if the sender is a player.
    #[must_use]
    pub fn get_player(&self) -> Option<&Arc<Player>> {
        match self {
            Self::Player(player) => Some(player),
            Self::SuppressedOutput(sender) => sender.get_player(),
            _ => None,
        }
    }

    /// Returns this sender with command output suppressed.
    #[must_use]
    pub fn with_suppressed_output(self) -> Self {
        if self.is_output_suppressed() {
            return self;
        }
        Self::SuppressedOutput(Arc::new(self))
    }

    /// Returns whether command output is suppressed.
    #[must_use]
    pub fn is_output_suppressed(&self) -> bool {
        matches!(self, Self::SuppressedOutput(_))
    }

    /// Sends a system message to the command sender.
    pub fn send_message(&self, text: &TextComponent) {
        match self {
            Self::Player(player) => player.send_message(text),
            Self::Console => log::info!("{text}"),
            Self::Rcon => log::warn!("Dropping Rcon command message until Rcon output is wired"),
            Self::SuppressedOutput(_) => {}
        }
    }

    pub(crate) fn send_failure(&self, message: impl Into<TextComponent>) {
        let message = TextComponent::new()
            .color(Color::Red)
            .add_child(message.into());
        self.send_message(&message);
    }

    pub(crate) fn send_failure_feedback(&self, feedback: CommandErrorFeedback) {
        for message in feedback.into_messages() {
            self.send_failure(message);
        }
    }
}

impl fmt::Display for CommandSender {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Player(p) => &p.gameprofile.name,
                Self::Console => "Server",
                Self::Rcon => "Rcon",
                Self::SuppressedOutput(sender) => return sender.fmt(f),
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::CommandSender;

    #[test]
    fn suppressed_output_keeps_source_identity() {
        let sender = CommandSender::Console.with_suppressed_output();

        assert!(sender.is_output_suppressed());
        assert_eq!(sender.to_string(), "Server");
    }

    #[test]
    fn suppressed_output_does_not_double_wrap() {
        let sender = CommandSender::Rcon.with_suppressed_output();
        let sender = sender.with_suppressed_output();

        let CommandSender::SuppressedOutput(inner) = sender else {
            panic!("sender should be output suppressed");
        };
        assert!(!matches!(&*inner, CommandSender::SuppressedOutput(_)));
    }
}
