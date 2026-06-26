use crate::command::context::CommandContext;
use crate::command::error::CommandError;
use crate::command::graph::PermissionTarget;
use crate::permission::PermissionSet;
use crate::player::player_data_storage::GlobalPlayerData;

pub(super) struct PermissionTargetState {
    pub(super) groups: Vec<String>,
    pub(super) overrides: PermissionSet,
}

pub(super) async fn load_state(
    context: &CommandContext,
    target: &PermissionTarget,
) -> Result<PermissionTargetState, CommandError> {
    if let Some(player) = target.online_player() {
        return Ok(PermissionTargetState {
            groups: player.permission_groups(),
            overrides: player.permission_overrides(),
        });
    }

    let data = context
        .server
        .player_data_storage
        .load_global(target.uuid())
        .await
        .map_err(|error| CommandError::failure(error.to_string()))?;

    let GlobalPlayerData {
        groups,
        permissions,
        ..
    } = data.unwrap_or_else(|| GlobalPlayerData {
        last_active_domain: context.server.worlds.default_domain().to_owned(),
        groups: Vec::new(),
        permissions: PermissionSet::default(),
    });

    Ok(PermissionTargetState {
        groups,
        overrides: permissions,
    })
}

pub(super) async fn save_state(
    context: &CommandContext,
    target: &PermissionTarget,
    state: PermissionTargetState,
) -> Result<(), CommandError> {
    if let Some(player) = target.online_player() {
        context
            .server
            .update_player_global_permissions(player, state.groups, state.overrides)
            .map_err(|error| CommandError::failure(error.to_string()))?;
        return Ok(());
    }

    context
        .server
        .update_offline_player_global_permissions(target.uuid(), state.groups, state.overrides)
        .await
        .map_err(|error| CommandError::failure(error.to_string()))
}
