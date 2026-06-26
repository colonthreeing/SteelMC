use std::sync::Arc;

use steel_utils::locks::SyncMutex;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;

use crate::command::sender::CommandSender;
use crate::player::connection::NetworkConnection;
use crate::server::Server;

pub(crate) struct CommandQueue {
    sender: UnboundedSender<QueuedCommand>,
    receiver: SyncMutex<Option<UnboundedReceiver<QueuedCommand>>>,
}

struct QueuedCommand {
    sender: CommandSender,
    command: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CommandQueueClosed;

impl CommandQueue {
    pub(crate) fn new() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self {
            sender,
            receiver: SyncMutex::new(Some(receiver)),
        }
    }

    pub(crate) fn submit(
        &self,
        sender: CommandSender,
        command: String,
    ) -> Result<(), CommandQueueClosed> {
        self.sender
            .send(QueuedCommand { sender, command })
            .map_err(|_| CommandQueueClosed)
    }

    pub(crate) fn start(&self, server: Arc<Server>, cancel_token: CancellationToken) {
        let Some(receiver) = self.receiver.lock().take() else {
            return;
        };
        tokio::spawn(run_commands(server, cancel_token, receiver));
    }
}

impl Default for CommandQueue {
    fn default() -> Self {
        Self::new()
    }
}

async fn run_commands(
    server: Arc<Server>,
    cancel_token: CancellationToken,
    mut receiver: UnboundedReceiver<QueuedCommand>,
) {
    loop {
        tokio::select! {
            () = cancel_token.cancelled() => break,
            maybe_command = receiver.recv() => {
                let Some(command) = maybe_command else {
                    break;
                };
                execute_command(&server, command).await;
            }
        }
    }
}

async fn execute_command(server: &Arc<Server>, command: QueuedCommand) {
    if sender_is_closed_player(&command.sender) {
        return;
    }

    let dispatcher = server.command_dispatcher.read().clone();
    dispatcher
        .handle_command(command.sender, command.command, server)
        .await;
}

fn sender_is_closed_player(sender: &CommandSender) -> bool {
    let CommandSender::Player(player) = sender else {
        return false;
    };
    player.connection.closed()
}
