use std::{collections::VecDeque, sync::Arc};

use steel_utils::locks::SyncMutex;

use crate::command::sender::CommandSender;
use crate::player::connection::NetworkConnection;
use crate::server::Server;

const DEFAULT_COMMAND_QUEUE_CAPACITY: usize = 1024;

pub(crate) struct CommandQueue {
    queued: SyncMutex<VecDeque<QueuedCommand>>,
    capacity: usize,
}

struct QueuedCommand {
    sender: CommandSender,
    command: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CommandQueueFull;

impl CommandQueue {
    pub(crate) fn new() -> Self {
        Self {
            queued: SyncMutex::new(VecDeque::new()),
            capacity: DEFAULT_COMMAND_QUEUE_CAPACITY,
        }
    }

    pub(crate) fn submit(
        &self,
        sender: CommandSender,
        command: String,
    ) -> Result<(), CommandQueueFull> {
        let mut queued = self.queued.lock();
        if queued.len() >= self.capacity {
            return Err(CommandQueueFull);
        }

        queued.push_back(QueuedCommand { sender, command });
        Ok(())
    }

    pub(crate) fn tick(&self, server: &Arc<Server>, max_commands: usize) -> usize {
        let mut handled = 0;
        for _ in 0..max_commands {
            let Some(command) = self.pop_front() else {
                break;
            };
            execute_command(server, command);
            handled += 1;
        }
        handled
    }

    pub(crate) fn clear(&self) {
        self.queued.lock().clear();
    }

    fn pop_front(&self) -> Option<QueuedCommand> {
        self.queued.lock().pop_front()
    }
}

impl Default for CommandQueue {
    fn default() -> Self {
        Self::new()
    }
}

fn execute_command(server: &Arc<Server>, command: QueuedCommand) {
    if sender_is_closed_player(&command.sender) {
        return;
    }

    let dispatcher = server.command_dispatcher.read().clone();
    dispatcher.handle_command(command.sender, command.command, server);
}

fn sender_is_closed_player(sender: &CommandSender) -> bool {
    let CommandSender::Player(player) = sender else {
        return false;
    };
    player.connection.closed()
}
