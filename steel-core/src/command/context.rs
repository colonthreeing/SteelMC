//! This module contains the command context.
use std::sync::Arc;

use glam::DVec3;

use crate::command::sender::CommandSender;
use crate::entity::Entity;
use crate::permission::{
    PermissionCatalog, PermissionContext, PermissionContextCatalog, PermissionMetadataCatalog,
};
use crate::player::Player;
use crate::server::Server;
use crate::world::World;

/// The context of a command.
#[derive(Clone)]
pub struct CommandContext {
    /// The sender of the command.
    pub sender: CommandSender,
    /// The player targeted by the command.
    pub player: Option<Arc<Player>>,
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
        let position = player
            .as_ref()
            // TODO: Check this. The default position is the surface of the world center
            // (Where the compass should point to)
            .map_or(
                DVec3::new(
                    f64::from(world_spawn.x),
                    f64::from(world_spawn.y),
                    f64::from(world_spawn.z),
                ),
                |p| p.position(),
            );

        let rotation = player.as_ref().map_or((0.0, 0.0), |p| p.rotation());

        Self {
            sender,
            player,
            world,
            server,
            position,
            rotation: Some(rotation),
            anchor: EntityAnchor::default(),
            permission_catalog: None,
            permission_metadata_catalog: None,
            permission_context_catalog: None,
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
}
