//! Handler for the "gamemode" command.
use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::{
    CommandNodeBuilder, CommandPermissionArgument, CommandResult, ParsedArguments, argument,
    literal,
};
use crate::command::parsers::{GameModeParser, PlayerParser};
use crate::command::requirement::RequirementContext;
use crate::command::{
    CommandRegistrationSpec, minecraft_command_permission_key,
};
use crate::entity::Entity;
use crate::permission::{PermissionExpr, PermissionKey, PermissionKeyError};
use crate::player::Player;
use std::sync::Arc;
use steel_utils::translations;
use steel_utils::types::GameType;
use text_components::TextComponent;
use text_components::translation::Translation;

pub(crate) const REGISTRATION: CommandRegistrationSpec = CommandRegistrationSpec::minecraft();

/// Handler for the "gamemode" command.
#[must_use]
pub(crate) fn command() -> CommandNodeBuilder {
    literal("gamemode").then(
        argument("gamemode", GameModeParser)
            .requires_argument_permission::<GameType>("gamemode")
            .executes(set_own_game_mode)
            .then(argument("targets", PlayerParser::multiple()).executes(set_target_game_mode)),
    )
}

pub(crate) fn can_use_client_gamemode_switcher(context: &dyn RequirementContext) -> bool {
    context_allows_builtin_permission(context, client_gamemode_switcher_permission())
}

pub(crate) fn can_change_game_mode(context: &dyn RequirementContext, game_mode: GameType) -> bool {
    context_allows_builtin_permission(context, change_game_mode_permission(game_mode))
}

fn context_allows_builtin_permission(
    context: &dyn RequirementContext,
    permission: Result<PermissionExpr, PermissionKeyError>,
) -> bool {
    match permission {
        Ok(permission) => context.has_permission(&permission),
        Err(error) => {
            log::error!("invalid built-in gamemode permission key: {error}");
            false
        }
    }
}

fn client_gamemode_switcher_permission() -> Result<PermissionExpr, PermissionKeyError> {
    let root = gamemode_root_permission()?;
    let game_modes = [
        GameType::Survival,
        GameType::Creative,
        GameType::Adventure,
        GameType::Spectator,
    ];
    let mode_permissions = game_modes
        .into_iter()
        .map(|game_mode| {
            gamemode_value_permission(&root, game_mode)
                .map(|mode| PermissionExpr::scoped_key(root.clone(), mode))
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(PermissionExpr::Any(mode_permissions))
}

fn change_game_mode_permission(game_mode: GameType) -> Result<PermissionExpr, PermissionKeyError> {
    let root = gamemode_root_permission()?;
    let mode = gamemode_value_permission(&root, game_mode)?;
    Ok(PermissionExpr::scoped_key(root, mode))
}

fn gamemode_root_permission() -> Result<PermissionKey, PermissionKeyError> {
    minecraft_command_permission_key("gamemode")
}

fn gamemode_value_permission(
    root: &PermissionKey,
    game_mode: GameType,
) -> Result<PermissionKey, PermissionKeyError> {
    root.child(&game_mode.permission_segment()?)
}

fn set_own_game_mode(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let gamemode = arguments
        .get::<GameType>("gamemode")
        .map_err(super::invalid_parsed_argument)?;

    let player = context
        .sender
        .get_player()
        .ok_or(CommandError::InvalidRequirement)?;

    player.set_game_mode(gamemode);

    Ok(CommandResult::success())
}

fn set_target_game_mode(
    context: &mut CommandContext,
    arguments: &ParsedArguments,
) -> Result<CommandResult, CommandError> {
    let gamemode = arguments
        .get::<GameType>("gamemode")
        .map_err(super::invalid_parsed_argument)?;
    let targets = arguments
        .get::<Vec<Arc<Player>>>("targets")
        .map_err(super::invalid_parsed_argument)?;

    let mode_translation = get_gamemode_translation(gamemode);

    for target in targets {
        if target.set_game_mode(gamemode) {
            let sender_is_target = if let Some(sender_player) = context.sender.get_player() {
                sender_player.id() == target.id()
            } else {
                false
            };

            if !sender_is_target {
                context.sender.send_message(
                    &translations::COMMANDS_GAMEMODE_SUCCESS_OTHER
                        .message([
                            TextComponent::plain(target.gameprofile.name.clone()),
                            TextComponent::from(mode_translation),
                        ])
                        .into(),
                );
            }
        }
    }

    Ok(CommandResult::success())
}

/// Retrieves the translation for a `GameType`
#[must_use]
pub fn get_gamemode_translation(gamemode: GameType) -> &'static Translation<0> {
    match gamemode {
        GameType::Survival => &translations::GAME_MODE_SURVIVAL,
        GameType::Creative => &translations::GAME_MODE_CREATIVE,
        GameType::Adventure => &translations::GAME_MODE_ADVENTURE,
        GameType::Spectator => &translations::GAME_MODE_SPECTATOR,
    }
}

#[cfg(test)]
mod tests {
    use super::{can_change_game_mode, can_use_client_gamemode_switcher};
    use crate::command::requirement::{CommandSourceKind, RequirementContext};
    use crate::permission::{PermissionEntry, PermissionExpr, PermissionKey, PermissionSet};
    use steel_utils::types::GameType;

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

    fn context_with_entries(entries: impl IntoIterator<Item = PermissionEntry>) -> TestContext {
        TestContext {
            permissions: PermissionSet::from_entries(entries),
        }
    }

    fn allow(permission: &'static str) -> PermissionEntry {
        PermissionEntry::allow(
            PermissionKey::parse(permission).expect("test permission key parses"),
        )
    }

    fn deny(permission: &'static str) -> PermissionEntry {
        PermissionEntry::deny(PermissionKey::parse(permission).expect("test permission key parses"))
    }

    #[test]
    fn root_and_one_game_mode_allow_client_switcher() {
        let context = context([
            "minecraft.command.gamemode",
            "minecraft.command.gamemode.creative",
        ]);

        assert!(can_use_client_gamemode_switcher(&context));
    }

    #[test]
    fn global_wildcard_allows_client_switcher_and_game_mode_changes() {
        let context = context(["*"]);

        assert!(can_use_client_gamemode_switcher(&context));
        assert!(can_change_game_mode(&context, GameType::Creative));
        assert!(can_change_game_mode(&context, GameType::Survival));
    }

    #[test]
    fn root_permission_alone_allows_client_switcher() {
        let context = context(["minecraft.command.gamemode"]);

        assert!(can_use_client_gamemode_switcher(&context));
    }

    #[test]
    fn game_mode_permission_without_root_allows_client_switcher() {
        let context = context(["minecraft.command.gamemode.creative"]);

        assert!(can_use_client_gamemode_switcher(&context));
    }

    #[test]
    fn root_permission_allows_every_game_mode_change() {
        let context = context(["minecraft.command.gamemode"]);

        assert!(can_change_game_mode(&context, GameType::Creative));
        assert!(can_change_game_mode(&context, GameType::Survival));
    }

    #[test]
    fn specific_game_mode_permission_only_allows_requested_mode() {
        let context = context(["minecraft.command.gamemode.creative"]);

        assert!(can_change_game_mode(&context, GameType::Creative));
        assert!(!can_change_game_mode(&context, GameType::Survival));
    }

    #[test]
    fn specific_game_mode_deny_overrides_root_permission() {
        let context = context_with_entries([
            allow("minecraft.command.gamemode"),
            deny("minecraft.command.gamemode.creative"),
        ]);

        assert!(can_use_client_gamemode_switcher(&context));
        assert!(!can_change_game_mode(&context, GameType::Creative));
        assert!(can_change_game_mode(&context, GameType::Survival));
    }

    #[test]
    fn client_switcher_rejects_when_every_game_mode_is_denied() {
        let context = context_with_entries([
            allow("minecraft.command.gamemode"),
            deny("minecraft.command.gamemode.survival"),
            deny("minecraft.command.gamemode.creative"),
            deny("minecraft.command.gamemode.adventure"),
            deny("minecraft.command.gamemode.spectator"),
        ]);

        assert!(!can_use_client_gamemode_switcher(&context));
    }
}
