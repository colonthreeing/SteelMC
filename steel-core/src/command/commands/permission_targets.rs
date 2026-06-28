use crate::command::error::CommandError;
use crate::command::graph::PermissionTarget;
use crate::permission::{PermissionSet, PermissionValueSet};
use crate::player::Player;
use crate::player::player_data_storage::GlobalPlayerData;
use crate::server::Server;
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct PermissionTargetState {
    pub(super) groups: Vec<String>,
    pub(super) overrides: PermissionSet,
    pub(super) value_overrides: PermissionValueSet,
}

pub(super) struct LoadedPermissionTargetState {
    target: PermissionTarget,
    source: PermissionTargetStateSource,
    state: PermissionTargetState,
}

enum PermissionTargetStateSource {
    Online(Arc<Player>),
    Offline,
}

impl LoadedPermissionTargetState {
    pub(super) fn target(&self) -> &PermissionTarget {
        &self.target
    }

    pub(super) fn state(&self) -> &PermissionTargetState {
        &self.state
    }

    pub(super) fn state_mut(&mut self) -> &mut PermissionTargetState {
        &mut self.state
    }
}

pub(super) async fn load_state(
    server: &Arc<Server>,
    target: PermissionTarget,
) -> Result<LoadedPermissionTargetState, CommandError> {
    let target = resolve(server, target).await?;
    if let Some((player, state)) = online_state(server, &target) {
        return Ok(LoadedPermissionTargetState {
            target,
            source: PermissionTargetStateSource::Online(player),
            state,
        });
    }

    let state = load_offline_state(server, &target).await?;
    Ok(LoadedPermissionTargetState {
        target,
        source: PermissionTargetStateSource::Offline,
        state,
    })
}

pub(super) async fn save_state(
    server: &Arc<Server>,
    loaded: LoadedPermissionTargetState,
) -> Result<(), CommandError> {
    let LoadedPermissionTargetState {
        target,
        source,
        state,
    } = loaded;
    match source {
        PermissionTargetStateSource::Online(player) => save_online_state(server, &player, state),
        PermissionTargetStateSource::Offline => save_offline_state(server, &target, state).await,
    }
}

async fn resolve(
    server: &Arc<Server>,
    target: PermissionTarget,
) -> Result<PermissionTarget, CommandError> {
    if target.is_resolved() {
        return Ok(target);
    }

    let profile = server
        .resolve_player_profile(target.name())
        .await
        .map_err(|error| CommandError::failure(error.to_string()))?;
    Ok(PermissionTarget::offline(
        profile.uuid(),
        profile.last_known_name().to_owned(),
    ))
}

async fn load_offline_state(
    server: &Arc<Server>,
    target: &PermissionTarget,
) -> Result<PermissionTargetState, CommandError> {
    let Some(uuid) = target.uuid() else {
        return Err(CommandError::failure(format!(
            "Player {} has not been resolved",
            target.name()
        )));
    };
    let data = server
        .player_data_storage
        .load_global(uuid)
        .await
        .map_err(|error| CommandError::failure(error.to_string()))?;

    let GlobalPlayerData {
        groups,
        permissions,
        values,
        ..
    } = data.unwrap_or_else(|| GlobalPlayerData {
        last_active_domain: server.worlds.default_domain().to_owned(),
        groups: Vec::new(),
        permissions: PermissionSet::default(),
        values: PermissionValueSet::default(),
    });

    Ok(PermissionTargetState {
        groups,
        overrides: permissions,
        value_overrides: values,
    })
}

pub(super) fn online_state(
    server: &Server,
    target: &PermissionTarget,
) -> Option<(Arc<Player>, PermissionTargetState)> {
    let player = server.get_player_by_uuid(&target.uuid()?)?;
    let state = PermissionTargetState {
        groups: player.permission_groups(),
        overrides: player.permission_overrides(),
        value_overrides: player.permission_value_overrides(),
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
    let state = server.global_permission_state(target.uuid()?)?;
    let (groups, overrides, value_overrides) = state.into_parts();
    Some(PermissionTargetState {
        groups,
        overrides,
        value_overrides,
    })
}

async fn save_offline_state(
    server: &Arc<Server>,
    target: &PermissionTarget,
    state: PermissionTargetState,
) -> Result<(), CommandError> {
    let Some(uuid) = target.uuid() else {
        return Err(CommandError::failure(format!(
            "Player {} has not been resolved",
            target.name()
        )));
    };
    server
        .update_offline_player_global_permissions(
            uuid,
            state.groups,
            state.overrides,
            state.value_overrides,
        )
        .await
        .map_err(|error| CommandError::failure(error.to_string()))
}

pub(super) fn save_online_state(
    server: &Arc<Server>,
    player: &Arc<Player>,
    state: PermissionTargetState,
) -> Result<(), CommandError> {
    server
        .update_player_global_permissions(
            player,
            state.groups,
            state.overrides,
            state.value_overrides,
        )
        .map_err(|error| CommandError::failure(error.to_string()))
}
