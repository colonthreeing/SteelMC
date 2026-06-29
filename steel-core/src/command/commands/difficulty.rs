//! Handler for the "difficulty" command

use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{CommandNodeBuilder, CommandResult, ParsedArguments, literal};
use crate::command::requirement::RequirementContext;
use crate::command::{
    CommandRegistrationSpec, minecraft_command_permission_key,
};
use crate::permission::{PermissionExpr, PermissionKeyError};
use steel_protocol::packets::game::CChangeDifficulty;
use steel_utils::translations;
use steel_utils::types::Difficulty;
use text_components::TextComponent;
use text_components::translation::Translation;

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Handler for the "difficulty" command
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
    literal("difficulty")
        .executes(query_difficulty)
        .then(difficulty_literal("peaceful", Difficulty::Peaceful))
        .then(difficulty_literal("easy", Difficulty::Easy))
        .then(difficulty_literal("normal", Difficulty::Normal))
        .then(difficulty_literal("hard", Difficulty::Hard))
}

fn difficulty_literal(name: &'static str, difficulty: Difficulty) -> CommandNodeBuilder {
    literal(name).executes(move |context: &mut CommandContext, _: &ParsedArguments| {
        set_difficulty(context, difficulty)
    })
}

pub(crate) fn can_change_difficulty(context: &dyn RequirementContext) -> bool {
    match difficulty_permission() {
        Ok(permission) => context.has_permission(&permission),
        Err(error) => {
            log::error!("invalid built-in difficulty permission key: {error}");
            false
        }
    }
}

fn difficulty_permission() -> Result<PermissionExpr, PermissionKeyError> {
    Ok(PermissionExpr::key(minecraft_command_permission_key(
        "difficulty",
    )?))
}

/// Returns the string key for a [`Difficulty`] variant
pub(crate) const fn difficulty_key(difficulty: Difficulty) -> &'static str {
    match difficulty {
        Difficulty::Peaceful => "peaceful",
        Difficulty::Easy => "easy",
        Difficulty::Normal => "normal",
        Difficulty::Hard => "hard",
    }
}

/// Returns the translatable display name for a [`Difficulty`] variant
fn difficulty_display_name(difficulty: Difficulty) -> &'static Translation<0> {
    match difficulty {
        Difficulty::Peaceful => &translations::OPTIONS_DIFFICULTY_PEACEFUL,
        Difficulty::Easy => &translations::OPTIONS_DIFFICULTY_EASY,
        Difficulty::Normal => &translations::OPTIONS_DIFFICULTY_NORMAL,
        Difficulty::Hard => &translations::OPTIONS_DIFFICULTY_HARD,
    }
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "Command executors use a shared fallible callback signature."
)]
fn query_difficulty(
    context: &mut CommandContext,
    _: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let difficulty = context.world.level_data.read().data().difficulty;
    let display_name = difficulty_display_name(difficulty);

    context.sender.send_message(
        &translations::COMMANDS_DIFFICULTY_QUERY
            .message([TextComponent::from(display_name)])
            .into(),
    );

    Ok(CommandResult::success())
}

#[cfg(test)]
mod tests {
    use super::can_change_difficulty;
    use crate::command::requirement::{CommandSourceKind, RequirementContext};
    use crate::permission::{PermissionEntry, PermissionExpr, PermissionKey, PermissionSet};

    struct TestContext {
        permissions: PermissionSet,
    }

    impl RequirementContext for TestContext {
        fn source_kind(&self) -> CommandSourceKind {
            CommandSourceKind::Player
        }

        fn has_permission(&self, permission: &PermissionExpr) -> bool {
            self.permissions.allows(permission)
        }
    }

    fn context(permissions: impl IntoIterator<Item = &'static str>) -> TestContext {
        TestContext {
            permissions: PermissionSet::from_entries(permissions.into_iter().map(|permission| {
                PermissionEntry::allow(
                    PermissionKey::parse(permission).expect("test permission key parses"),
                )
            })),
        }
    }

    #[test]
    fn root_permission_allows_client_difficulty_changes() {
        let context = context(["minecraft.command.difficulty"]);

        assert!(can_change_difficulty(&context));
    }

    #[test]
    fn missing_root_permission_rejects_client_difficulty_changes() {
        let context = context(["minecraft.command.gamemode"]);

        assert!(!can_change_difficulty(&context));
    }

    #[test]
    fn global_wildcard_allows_client_difficulty_changes() {
        let context = context(["*"]);

        assert!(can_change_difficulty(&context));
    }
}

fn set_difficulty(
    context: &mut CommandContext,
    difficulty: Difficulty,
) -> Result<CommandResult, CommandError> {
    let domain = context.world.domain().to_owned();
    let worlds = context.server.worlds.worlds_in_domain(&domain);

    if worlds
        .iter()
        .all(|world| world.level_data.read().data().difficulty == difficulty)
    {
        return Err(CommandError::failure(
            translations::COMMANDS_DIFFICULTY_FAILURE
                .message([TextComponent::plain(difficulty_key(difficulty))]),
        ));
    }

    for world in worlds {
        let mut level_data = world.level_data.write();
        level_data.data_mut().difficulty = difficulty;
        let locked = level_data.data().difficulty_locked;
        drop(level_data);

        world.broadcast_to_all(CChangeDifficulty { difficulty, locked });
    }

    let display_name = difficulty_display_name(difficulty);
    context.sender.send_message(
        &translations::COMMANDS_DIFFICULTY_SUCCESS
            .message([TextComponent::from(display_name)])
            .into(),
    );

    Ok(CommandResult::success())
}
