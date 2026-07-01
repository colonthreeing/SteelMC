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
    sender
        .get_player()
        .is_some_and(|player| player.connection.closed())
}

#[cfg(test)]
mod tests {
    use super::{CommandQueue, CommandQueueFull, QueuedCommand};
    use crate::command::sender::CommandSender;
    use std::collections::VecDeque;
    use steel_utils::locks::SyncMutex;

    fn queue_with_capacity(capacity: usize) -> CommandQueue {
        CommandQueue {
            queued: SyncMutex::new(VecDeque::new()),
            capacity,
        }
    }

    fn submit(queue: &CommandQueue, command: &str) -> Result<(), CommandQueueFull> {
        queue.submit(CommandSender::Console, command.to_owned())
    }

    fn pop_command(queue: &CommandQueue) -> Option<String> {
        queue
            .pop_front()
            .map(|QueuedCommand { command, .. }| command)
    }

    #[test]
    fn submit_preserves_fifo_order() {
        let queue = queue_with_capacity(3);

        assert!(submit(&queue, "say first").is_ok());
        assert!(submit(&queue, "say second").is_ok());

        assert_eq!(pop_command(&queue).as_deref(), Some("say first"));
        assert_eq!(pop_command(&queue).as_deref(), Some("say second"));
        assert_eq!(pop_command(&queue), None);
    }

    #[test]
    fn full_queue_rejects_without_dropping_existing_commands() {
        let queue = queue_with_capacity(2);

        assert!(submit(&queue, "say first").is_ok());
        assert!(submit(&queue, "say second").is_ok());
        assert_eq!(submit(&queue, "say third"), Err(CommandQueueFull));

        assert_eq!(pop_command(&queue).as_deref(), Some("say first"));
        assert_eq!(pop_command(&queue).as_deref(), Some("say second"));
        assert_eq!(pop_command(&queue), None);
    }

    #[test]
    fn clear_discards_pending_commands() {
        let queue = queue_with_capacity(2);

        assert!(submit(&queue, "say first").is_ok());
        assert!(submit(&queue, "say second").is_ok());

        queue.clear();

        assert_eq!(pop_command(&queue), None);
    }
}
