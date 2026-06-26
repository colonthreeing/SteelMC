use std::{fmt, sync::Arc};

use glam::DVec3;
use steel_registry::{
    enchantment::EnchantmentRef, entity_type::EntityTypeRef, items::ItemRef,
    structure::StructureRef,
};
use steel_utils::{BlockPos, Identifier, types::GameType};
use text_components::TextComponent;
use uuid::Uuid;

use crate::command::context::EntityAnchor;
use crate::entity::LivingEntity;
use crate::permission::{PermissionKey, PermissionKeyError, PermissionSegment};
use crate::player::Player;
use crate::world::World;

/// A parsed command argument value.
#[derive(Clone)]
pub enum ParsedArgument {
    /// Boolean argument.
    Bool(bool),
    /// Entity anchor argument.
    Anchor(EntityAnchor),
    /// 32-bit signed integer argument.
    I32(i32),
    /// 32-bit floating-point argument.
    F32(f32),
    /// String-like argument.
    String(String),
    /// Permission key argument.
    PermissionKey(PermissionKey),
    /// Game mode argument.
    GameMode(GameType),
    /// Player target argument.
    Players(Vec<Arc<Player>>),
    /// Permission-management player target argument.
    PermissionTargets(Vec<PermissionTarget>),
    /// Living entity target argument.
    Entities(Vec<Arc<dyn LivingEntity + Send + Sync>>),
    /// Entity type argument.
    EntityType(EntityTypeRef),
    /// Item argument.
    Item(ItemRef),
    /// Enchantment argument.
    Enchantment(EnchantmentRef),
    /// Structure or structure tag argument.
    Structure(StructureArgumentValue),
    /// Loaded world argument.
    World(Arc<World>),
    /// 3D vector argument.
    Vec3(DVec3),
    /// Block position argument.
    BlockPos(BlockPos),
    /// Rotation argument.
    Rotation((f32, f32)),
    /// Text component argument.
    Component(Box<TextComponent>),
}

/// Structure command argument value: either one structure or a structure tag.
#[derive(Clone, Debug)]
pub enum StructureArgumentValue {
    /// A single structure key.
    Structure(StructureRef),
    /// A structure tag and its resolved entries.
    Tag {
        /// Tag key without the leading `#`.
        key: Identifier,
        /// Structures in the tag.
        structures: Vec<StructureRef>,
    },
}

/// Player target for permission-management commands.
#[derive(Clone)]
pub struct PermissionTarget {
    uuid: Uuid,
    name: String,
    online_player: Option<Arc<Player>>,
}

impl PermissionTarget {
    /// Creates a target from an online player.
    #[must_use]
    pub fn online(player: Arc<Player>) -> Self {
        Self {
            uuid: player.gameprofile.id,
            name: player.gameprofile.name.clone(),
            online_player: Some(player),
        }
    }

    /// Creates a target from a known offline profile.
    #[must_use]
    pub fn offline(uuid: Uuid, name: impl Into<String>) -> Self {
        Self {
            uuid,
            name: name.into(),
            online_player: None,
        }
    }

    /// Returns the target UUID.
    #[must_use]
    pub const fn uuid(&self) -> Uuid {
        self.uuid
    }

    /// Returns the target display name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the online player when this target is connected.
    #[must_use]
    pub fn online_player(&self) -> Option<&Arc<Player>> {
        self.online_player.as_ref()
    }
}

impl fmt::Debug for PermissionTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PermissionTarget")
            .field("uuid", &self.uuid)
            .field("name", &self.name)
            .field("online", &self.online_player.is_some())
            .finish()
    }
}

impl StructureArgumentValue {
    /// Structure keys to scan.
    #[must_use]
    pub fn structure_keys(&self) -> Vec<Identifier> {
        match self {
            Self::Structure(structure) => vec![structure.key.clone()],
            Self::Tag { structures, .. } => structures
                .iter()
                .map(|structure| structure.key.clone())
                .collect(),
        }
    }

    /// Printable command target name.
    #[must_use]
    pub fn printable_name(&self, found_structure: &Identifier) -> String {
        match self {
            Self::Structure(structure) => structure.key.to_string(),
            Self::Tag { key, .. } => format!("#{key} ({found_structure})"),
        }
    }

    /// Printable command target without resolved found entry.
    #[must_use]
    pub fn query_name(&self) -> String {
        match self {
            Self::Structure(structure) => structure.key.to_string(),
            Self::Tag { key, .. } => format!("#{key}"),
        }
    }
}

impl fmt::Debug for ParsedArgument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(value) => f.debug_tuple("Bool").field(value).finish(),
            Self::Anchor(value) => f.debug_tuple("Anchor").field(value).finish(),
            Self::I32(value) => f.debug_tuple("I32").field(value).finish(),
            Self::F32(value) => f.debug_tuple("F32").field(value).finish(),
            Self::String(value) => f.debug_tuple("String").field(value).finish(),
            Self::PermissionKey(value) => f.debug_tuple("PermissionKey").field(value).finish(),
            Self::GameMode(value) => f.debug_tuple("GameMode").field(value).finish(),
            Self::Players(value) => f
                .debug_struct("Players")
                .field("count", &value.len())
                .finish(),
            Self::PermissionTargets(value) => f
                .debug_struct("PermissionTargets")
                .field("count", &value.len())
                .finish(),
            Self::Entities(value) => f
                .debug_struct("Entities")
                .field("count", &value.len())
                .finish(),
            Self::EntityType(value) => f.debug_tuple("EntityType").field(&value.key).finish(),
            Self::Item(value) => f.debug_tuple("Item").field(&value.key).finish(),
            Self::Enchantment(value) => f.debug_tuple("Enchantment").field(&value.key).finish(),
            Self::Structure(value) => f.debug_tuple("Structure").field(value).finish(),
            Self::World(value) => f.debug_tuple("World").field(&value.key).finish(),
            Self::Vec3(value) => f.debug_tuple("Vec3").field(value).finish(),
            Self::BlockPos(value) => f.debug_tuple("BlockPos").field(value).finish(),
            Self::Rotation(value) => f.debug_tuple("Rotation").field(value).finish(),
            Self::Component(_) => f.debug_tuple("Component").finish(),
        }
    }
}

/// Typed command argument storage.
#[derive(Clone, Debug, Default)]
pub struct ParsedArguments {
    values: Vec<ParsedArgumentEntry>,
}

#[derive(Clone, Debug)]
struct ParsedArgumentEntry {
    name: String,
    value: ParsedArgument,
}

impl ParsedArguments {
    /// Stores an argument value.
    pub fn insert(&mut self, name: impl Into<String>, value: ParsedArgument) {
        let name = name.into();
        if let Some(existing) = self.values.iter_mut().find(|entry| entry.name == name) {
            existing.value = value;
            return;
        }

        self.values.push(ParsedArgumentEntry { name, value });
    }

    /// Returns a typed argument by name.
    ///
    /// # Errors
    ///
    /// Returns an error when the argument is missing or has the wrong type.
    pub fn get<T: FromParsedArgument>(&self, name: &str) -> Result<T, ParsedArgumentError> {
        let value = self
            .values
            .iter()
            .rev()
            .find_map(|entry| (entry.name == name).then_some(&entry.value))
            .ok_or_else(|| ParsedArgumentError::Missing(name.to_owned()))?;

        T::from_parsed_argument(value).ok_or_else(|| ParsedArgumentError::WrongType {
            name: name.to_owned(),
            expected: T::TYPE_NAME,
            actual: value.type_name(),
        })
    }
}

impl ParsedArgument {
    const fn type_name(&self) -> &'static str {
        match self {
            Self::Bool(_) => "bool",
            Self::Anchor(_) => "anchor",
            Self::I32(_) => "i32",
            Self::F32(_) => "f32",
            Self::String(_) => "string",
            Self::PermissionKey(_) => "permission_key",
            Self::GameMode(_) => "gamemode",
            Self::Players(_) => "players",
            Self::PermissionTargets(_) => "permission_targets",
            Self::Entities(_) => "entities",
            Self::EntityType(_) => "entity_type",
            Self::Item(_) => "item",
            Self::Enchantment(_) => "enchantment",
            Self::Structure(_) => "structure",
            Self::World(_) => "world",
            Self::Vec3(_) => "vec3",
            Self::BlockPos(_) => "block_pos",
            Self::Rotation(_) => "rotation",
            Self::Component(_) => "component",
        }
    }
}

/// Conversion from a parsed argument.
pub trait FromParsedArgument: Sized {
    /// Expected type name for diagnostics.
    const TYPE_NAME: &'static str;

    /// Converts from a parsed argument if the type matches.
    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self>;
}

/// A parsed argument value that can become one permission path segment.
pub trait CommandPermissionArgument: FromParsedArgument {
    /// Converts this argument value to one permission segment.
    ///
    /// # Errors
    ///
    /// Returns an error if the argument value cannot form a valid permission
    /// segment.
    fn permission_segment(&self) -> Result<PermissionSegment, PermissionKeyError>;
}

impl FromParsedArgument for bool {
    const TYPE_NAME: &'static str = "bool";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Bool(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for EntityAnchor {
    const TYPE_NAME: &'static str = "anchor";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Anchor(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for i32 {
    const TYPE_NAME: &'static str = "i32";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::I32(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for f32 {
    const TYPE_NAME: &'static str = "f32";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::F32(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for String {
    const TYPE_NAME: &'static str = "string";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::String(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for PermissionKey {
    const TYPE_NAME: &'static str = "permission_key";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::PermissionKey(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for GameType {
    const TYPE_NAME: &'static str = "gamemode";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::GameMode(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl CommandPermissionArgument for GameType {
    fn permission_segment(&self) -> Result<PermissionSegment, PermissionKeyError> {
        PermissionSegment::parse(self.name())
    }
}

impl FromParsedArgument for Vec<Arc<Player>> {
    const TYPE_NAME: &'static str = "players";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Players(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for Vec<PermissionTarget> {
    const TYPE_NAME: &'static str = "permission_targets";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::PermissionTargets(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for Vec<Arc<dyn LivingEntity + Send + Sync>> {
    const TYPE_NAME: &'static str = "entities";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Entities(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for EntityTypeRef {
    const TYPE_NAME: &'static str = "entity_type";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::EntityType(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for ItemRef {
    const TYPE_NAME: &'static str = "item";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Item(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for EnchantmentRef {
    const TYPE_NAME: &'static str = "enchantment";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Enchantment(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for StructureArgumentValue {
    const TYPE_NAME: &'static str = "structure";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Structure(value) = value else {
            return None;
        };
        Some(value.clone())
    }
}

impl FromParsedArgument for Arc<World> {
    const TYPE_NAME: &'static str = "world";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::World(value) = value else {
            return None;
        };
        Some(Arc::clone(value))
    }
}

impl FromParsedArgument for DVec3 {
    const TYPE_NAME: &'static str = "vec3";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Vec3(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for BlockPos {
    const TYPE_NAME: &'static str = "block_pos";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::BlockPos(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for (f32, f32) {
    const TYPE_NAME: &'static str = "rotation";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Rotation(value) = value else {
            return None;
        };
        Some(*value)
    }
}

impl FromParsedArgument for TextComponent {
    const TYPE_NAME: &'static str = "component";

    fn from_parsed_argument(value: &ParsedArgument) -> Option<Self> {
        let ParsedArgument::Component(value) = value else {
            return None;
        };
        Some(value.as_ref().clone())
    }
}

/// Error returned when reading a typed parsed argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParsedArgumentError {
    /// The requested argument was not parsed.
    Missing(String),
    /// The requested argument has a different type.
    WrongType {
        /// Argument name.
        name: String,
        /// Expected type name.
        expected: &'static str,
        /// Actual type name.
        actual: &'static str,
    },
}
