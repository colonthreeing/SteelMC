//! This module contains the command context.
use std::sync::Arc;

use glam::DVec3;

use crate::command::sender::CommandSender;
use crate::entity::{Entity, SharedEntity};
use crate::permission::{
    PermissionCatalog, PermissionContext, PermissionContextCatalog, PermissionMetadataCatalog,
};
use crate::player::Player;
use crate::server::Server;
use crate::world::World;

/// Result reported to a command source callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandCallbackResult {
    /// Whether the command returned normally.
    pub success: bool,
    /// Integer result returned by the command.
    pub result: i32,
}

type CommandResultCallbackFn = dyn Fn(CommandCallbackResult) + Send + Sync;

/// Callback invoked after a command source executes a terminal command.
#[derive(Clone, Default)]
pub struct CommandResultCallback {
    callback: Option<Arc<CommandResultCallbackFn>>,
}

impl CommandResultCallback {
    /// Creates a command result callback.
    #[must_use]
    pub fn new(callback: impl Fn(CommandCallbackResult) + Send + Sync + 'static) -> Self {
        Self {
            callback: Some(Arc::new(callback)),
        }
    }

    /// Creates an empty callback.
    #[must_use]
    pub const fn empty() -> Self {
        Self { callback: None }
    }

    /// Returns a callback that invokes `self`, then `next`.
    #[must_use]
    pub fn chain(self, next: Self) -> Self {
        match (self.callback, next.callback) {
            (None, None) => Self::empty(),
            (Some(callback), None) | (None, Some(callback)) => Self {
                callback: Some(callback),
            },
            (Some(first), Some(second)) => Self::new(move |result| {
                first(result);
                second(result);
            }),
        }
    }

    pub(crate) fn on_result(&self, result: CommandCallbackResult) {
        if let Some(callback) = &self.callback {
            callback(result);
        }
    }
}

/// The context of a command.
#[derive(Clone)]
pub struct CommandContext {
    /// The sender of the command.
    pub sender: CommandSender,
    /// The player targeted by the command.
    pub player: Option<Arc<Player>>,
    /// The entity the command is executing as.
    pub entity: Option<SharedEntity>,
    /// The world the command is executing in.
    pub world: Arc<World>,
    /// The server where the command has been run.
    pub server: Arc<Server>,
    /// The position of the command.
    pub position: DVec3,
    /// The rotation of the command.
    pub rotation: Option<(f32, f32)>,
    /// The anchor of the command.
    pub anchor: EntityAnchor,
    permission_catalog: Option<PermissionCatalog>,
    permission_metadata_catalog: Option<PermissionMetadataCatalog>,
    permission_context_catalog: Option<PermissionContextCatalog>,
    result_callback: CommandResultCallback,
}

/// The position anchor to use for an entity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EntityAnchor {
    /// The feet of the entity.
    #[default]
    Feet,
    /// The eyes of the entity.
    Eyes,
}

impl CommandContext {
    /// Creates a new command context.
    #[must_use]
    pub fn new(sender: CommandSender, server: Arc<Server>) -> Self {
        let player = sender.get_player().cloned();
        let world = player
            .as_ref()
            .map_or(server.overworld().clone(), |p| p.get_world());
        let world_spawn = world.level_data.read().data().spawn.clone();
        let entity = player
            .as_ref()
            .map(|player| Arc::clone(player) as SharedEntity);
        let position = entity
            .as_ref()
            // Vanilla uses the lower corner of the server respawn block for non-entity sources.
            .map_or(
                DVec3::new(
                    f64::from(world_spawn.x),
                    f64::from(world_spawn.y),
                    f64::from(world_spawn.z),
                ),
                |entity| entity.position(),
            );

        let rotation = entity
            .as_ref()
            .map_or((0.0, 0.0), |entity| entity.rotation());

        Self {
            sender,
            player,
            entity,
            world,
            server,
            position,
            rotation: Some(rotation),
            anchor: EntityAnchor::default(),
            permission_catalog: None,
            permission_metadata_catalog: None,
            permission_context_catalog: None,
            result_callback: CommandResultCallback::empty(),
        }
    }

    /// Adds a permission catalog available to command suggestion providers.
    #[must_use]
    pub fn with_permission_catalog(mut self, catalog: PermissionCatalog) -> Self {
        self.permission_catalog = Some(catalog);
        self
    }

    /// Adds a permission metadata catalog available to command suggestion providers.
    #[must_use]
    pub fn with_permission_metadata_catalog(mut self, catalog: PermissionMetadataCatalog) -> Self {
        self.permission_metadata_catalog = Some(catalog);
        self
    }

    /// Adds a permission context catalog available to command suggestion providers.
    #[must_use]
    pub fn with_permission_context_catalog(mut self, catalog: PermissionContextCatalog) -> Self {
        self.permission_context_catalog = Some(catalog);
        self
    }

    pub(crate) fn permission_catalog(&self) -> Option<&PermissionCatalog> {
        self.permission_catalog.as_ref()
    }

    pub(crate) fn permission_metadata_catalog(&self) -> Option<&PermissionMetadataCatalog> {
        self.permission_metadata_catalog.as_ref()
    }

    pub(crate) fn permission_context_catalog(&self) -> Option<&PermissionContextCatalog> {
        self.permission_context_catalog.as_ref()
    }

    pub(crate) fn permission_check_context(&self) -> PermissionContext {
        PermissionContext::for_world(self.world.domain().to_owned(), self.world.key.clone())
    }

    /// Returns this context with a replaced result callback.
    #[must_use]
    pub fn with_result_callback(mut self, callback: CommandResultCallback) -> Self {
        self.result_callback = callback;
        self
    }

    /// Returns this context with `callback` chained after the current result callback.
    #[must_use]
    pub fn with_chained_result_callback(mut self, callback: CommandResultCallback) -> Self {
        self.result_callback = self.result_callback.chain(callback);
        self
    }

    /// Chains `callback` after the current result callback.
    pub fn chain_result_callback(&mut self, callback: CommandResultCallback) {
        let current = std::mem::take(&mut self.result_callback);
        self.result_callback = current.chain(callback);
    }

    /// Returns this context without result callbacks.
    #[must_use]
    pub fn without_result_callbacks(mut self) -> Self {
        self.result_callback = CommandResultCallback::empty();
        self
    }

    /// Returns this context with command output suppressed.
    #[must_use]
    pub fn with_suppressed_output(mut self) -> Self {
        self.sender = self.sender.with_suppressed_output();
        self
    }

    /// Returns whether command output is suppressed.
    #[must_use]
    pub fn is_output_suppressed(&self) -> bool {
        self.sender.is_output_suppressed()
    }

    pub(crate) fn on_command_result(&self, result: CommandCallbackResult) {
        self.result_callback.on_result(result);
    }

    /// Returns this context with a different source entity.
    #[must_use]
    pub fn with_entity(mut self, entity: SharedEntity) -> Self {
        self.player = self
            .server
            .get_players()
            .into_iter()
            .find(|player| player.uuid() == entity.uuid());
        self.entity = Some(entity);
        self
    }

    /// Returns this context with a different execution world.
    #[must_use]
    pub fn with_world(mut self, world: Arc<World>) -> Self {
        if self.world.key != world.key {
            let scale =
                self.world.dimension_type.coordinate_scale / world.dimension_type.coordinate_scale;
            self.position.x *= scale;
            self.position.z *= scale;
        }
        self.world = world;
        self
    }

    /// Returns this context with a different execution position.
    #[must_use]
    pub fn with_position(mut self, position: DVec3) -> Self {
        self.position = position;
        self
    }

    /// Returns this context with a different execution rotation.
    #[must_use]
    pub fn with_rotation(mut self, rotation: (f32, f32)) -> Self {
        self.rotation = Some(normalize_rotation(rotation));
        self
    }

    /// Returns this context with a different local-coordinate anchor.
    #[must_use]
    pub const fn with_anchor(mut self, anchor: EntityAnchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// Returns this context facing `target`.
    #[must_use]
    pub fn facing_position(self, target: DVec3) -> Self {
        let from = self.anchor_position();
        let delta = target - from;
        let horizontal = delta.x.hypot(delta.z);
        let pitch = -delta.y.atan2(horizontal).to_degrees() as f32;
        let yaw = delta.z.atan2(delta.x).to_degrees() as f32 - 90.0;
        self.with_rotation((yaw, pitch))
    }

    /// Returns the current source position after applying the command anchor.
    #[must_use]
    pub fn anchor_position(&self) -> DVec3 {
        anchored_position(self.position, self.entity.as_deref(), self.anchor)
    }
}

/// Returns the position of `entity` after applying `anchor`.
#[must_use]
pub fn anchored_position(
    position: DVec3,
    entity: Option<&dyn Entity>,
    anchor: EntityAnchor,
) -> DVec3 {
    if matches!(anchor, EntityAnchor::Eyes)
        && let Some(entity) = entity
    {
        return DVec3::new(position.x, position.y + entity.get_eye_height(), position.z);
    }

    position
}

fn normalize_rotation((mut yaw, mut pitch): (f32, f32)) -> (f32, f32) {
    yaw = yaw.rem_euclid(360.0);
    if yaw >= 180.0 {
        yaw -= 360.0;
    }
    pitch = pitch.rem_euclid(360.0);
    if pitch >= 180.0 {
        pitch -= 360.0;
    }

    (yaw, pitch)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::{CommandCallbackResult, CommandResultCallback};

    #[test]
    fn command_result_callbacks_chain_in_order() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let first_events = Arc::clone(&events);
        let second_events = Arc::clone(&events);

        let first = CommandResultCallback::new(move |result| {
            first_events
                .lock()
                .expect("events lock should not be poisoned")
                .push(("first", result));
        });
        let second = CommandResultCallback::new(move |result| {
            second_events
                .lock()
                .expect("events lock should not be poisoned")
                .push(("second", result));
        });

        first.chain(second).on_result(CommandCallbackResult {
            success: true,
            result: 7,
        });

        assert_eq!(
            *events.lock().expect("events lock should not be poisoned"),
            vec![
                (
                    "first",
                    CommandCallbackResult {
                        success: true,
                        result: 7
                    }
                ),
                (
                    "second",
                    CommandCallbackResult {
                        success: true,
                        result: 7
                    }
                ),
            ]
        );
    }
}
