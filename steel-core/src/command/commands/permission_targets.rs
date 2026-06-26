use crate::command::error::CommandError;
use crate::command::graph::PermissionTarget;
use crate::permission::PermissionSet;
use crate::player::Player;
use crate::player::player_data_storage::GlobalPlayerData;
use crate::server::Server;
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct PermissionTargetState {
    pub(super) groups: Vec<String>,
    pub(super) overrides: PermissionSet,
}

pub(super) async fn load_offline_state(
    server: &Arc<Server>,
    target: &PermissionTarget,
) -> Result<PermissionTargetState, CommandError> {
    let data = server
        .player_data_storage
        .load_global(target.uuid())
        .await
        .map_err(|error| CommandError::failure(error.to_string()))?;

    let GlobalPlayerData {
        groups,
        permissions,
        ..
    } = data.unwrap_or_else(|| GlobalPlayerData {
        last_active_domain: server.worlds.default_domain().to_owned(),
        groups: Vec::new(),
        permissions: PermissionSet::default(),
    });

    Ok(PermissionTargetState {
        groups,
        overrides: permissions,
    })
}

pub(super) fn online_state(
    server: &Server,
    target: &PermissionTarget,
) -> Option<(Arc<Player>, PermissionTargetState)> {
    let player = server.get_player_by_uuid(&target.uuid())?;
    let state = PermissionTargetState {
        groups: player.permission_groups(),
        overrides: player.permission_overrides(),
    };
    Some((player, state))
}

pub(super) fn cached_state(
    server: &Server,
    target: &PermissionTarget,
) -> Option<PermissionTargetState> {
    if let Some((_, state)) = online_state(server, target) {
        return Some(state);
    }
    let state = server.global_permission_state(target.uuid())?;
    let (groups, overrides) = state.into_parts();
    Some(PermissionTargetState { groups, overrides })
}

pub(super) async fn save_offline_state(
    server: &Arc<Server>,
    target: &PermissionTarget,
    state: PermissionTargetState,
) -> Result<(), CommandError> {
    server
        .update_offline_player_global_permissions(target.uuid(), state.groups, state.overrides)
        .await
        .map_err(|error| CommandError::failure(error.to_string()))
}

pub(super) fn save_online_state(
    server: &Arc<Server>,
    player: &Arc<Player>,
    state: PermissionTargetState,
) -> Result<(), CommandError> {
    server
        .update_player_global_permissions(player, state.groups, state.overrides)
        .map_err(|error| CommandError::failure(error.to_string()))
}
