//! Clientbound boss event packet.

use std::io::{Result, Write};

use steel_macros::ClientPacket;
use steel_registry::packets::play::C_BOSS_EVENT;
use steel_utils::{codec::VarInt, serial::WriteTo};
use text_components::TextComponent;
use uuid::Uuid;

/// Updates one boss bar on the client.
#[derive(ClientPacket, Clone, Debug)]
#[packet_id(Play = C_BOSS_EVENT)]
pub struct CBossEvent {
    /// Runtime boss event id.
    pub id: Uuid,
    /// Boss event operation.
    pub operation: BossEventOperation,
}

impl CBossEvent {
    /// Creates an add operation.
    #[must_use]
    pub fn add(
        id: Uuid,
        name: TextComponent,
        progress: f32,
        color: BossBarColor,
        overlay: BossBarOverlay,
        properties: BossBarProperties,
    ) -> Self {
        Self {
            id,
            operation: BossEventOperation::Add {
                name,
                progress,
                color,
                overlay,
                properties,
            },
        }
    }

    /// Creates a remove operation.
    #[must_use]
    pub const fn remove(id: Uuid) -> Self {
        Self {
            id,
            operation: BossEventOperation::Remove,
        }
    }

    /// Creates a progress update operation.
    #[must_use]
    pub fn update_progress(id: Uuid, progress: f32) -> Self {
        Self {
            id,
            operation: BossEventOperation::UpdateProgress { progress },
        }
    }

    /// Creates a name update operation.
    #[must_use]
    pub fn update_name(id: Uuid, name: TextComponent) -> Self {
        Self {
            id,
            operation: BossEventOperation::UpdateName { name },
        }
    }

    /// Creates a style update operation.
    #[must_use]
    pub fn update_style(id: Uuid, color: BossBarColor, overlay: BossBarOverlay) -> Self {
        Self {
            id,
            operation: BossEventOperation::UpdateStyle { color, overlay },
        }
    }

    /// Creates a property update operation.
    #[must_use]
    pub fn update_properties(id: Uuid, properties: BossBarProperties) -> Self {
        Self {
            id,
            operation: BossEventOperation::UpdateProperties { properties },
        }
    }
}

impl WriteTo for CBossEvent {
    fn write(&self, writer: &mut impl Write) -> Result<()> {
        self.id.write(writer)?;
        VarInt(self.operation.discriminant()).write(writer)?;
        self.operation.write_payload(writer)
    }
}

/// Boss bar operation.
#[derive(Clone, Debug)]
pub enum BossEventOperation {
    /// Adds a boss bar.
    Add {
        /// Display name.
        name: TextComponent,
        /// Progress from 0.0 to 1.0.
        progress: f32,
        /// Bar color.
        color: BossBarColor,
        /// Bar overlay.
        overlay: BossBarOverlay,
        /// Screen effect properties.
        properties: BossBarProperties,
    },
    /// Removes a boss bar.
    Remove,
    /// Updates progress.
    UpdateProgress {
        /// Progress from 0.0 to 1.0.
        progress: f32,
    },
    /// Updates display name.
    UpdateName {
        /// Display name.
        name: TextComponent,
    },
    /// Updates style.
    UpdateStyle {
        /// Bar color.
        color: BossBarColor,
        /// Bar overlay.
        overlay: BossBarOverlay,
    },
    /// Updates screen effect properties.
    UpdateProperties {
        /// Screen effect properties.
        properties: BossBarProperties,
    },
}

impl BossEventOperation {
    const fn discriminant(&self) -> i32 {
        match self {
            Self::Add { .. } => 0,
            Self::Remove => 1,
            Self::UpdateProgress { .. } => 2,
            Self::UpdateName { .. } => 3,
            Self::UpdateStyle { .. } => 4,
            Self::UpdateProperties { .. } => 5,
        }
    }

    fn write_payload(&self, writer: &mut impl Write) -> Result<()> {
        match self {
            Self::Add {
                name,
                progress,
                color,
                overlay,
                properties,
            } => {
                name.write(writer)?;
                progress.write(writer)?;
                color.write(writer)?;
                overlay.write(writer)?;
                properties.write(writer)
            }
            Self::Remove => Ok(()),
            Self::UpdateProgress { progress } => progress.write(writer),
            Self::UpdateName { name } => name.write(writer),
            Self::UpdateStyle { color, overlay } => {
                color.write(writer)?;
                overlay.write(writer)
            }
            Self::UpdateProperties { properties } => properties.write(writer),
        }
    }
}

/// Boss bar color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BossBarColor {
    /// Pink bar.
    Pink,
    /// Blue bar.
    Blue,
    /// Red bar.
    Red,
    /// Green bar.
    Green,
    /// Yellow bar.
    Yellow,
    /// Purple bar.
    Purple,
    /// White bar.
    White,
}

impl BossBarColor {
    const fn discriminant(self) -> i32 {
        match self {
            Self::Pink => 0,
            Self::Blue => 1,
            Self::Red => 2,
            Self::Green => 3,
            Self::Yellow => 4,
            Self::Purple => 5,
            Self::White => 6,
        }
    }

    /// Returns the vanilla serialized name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pink => "pink",
            Self::Blue => "blue",
            Self::Red => "red",
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Purple => "purple",
            Self::White => "white",
        }
    }

    /// Parses a vanilla boss bar color name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "pink" => Self::Pink,
            "blue" => Self::Blue,
            "red" => Self::Red,
            "green" => Self::Green,
            "yellow" => Self::Yellow,
            "purple" => Self::Purple,
            "white" => Self::White,
            _ => return None,
        })
    }
}

impl WriteTo for BossBarColor {
    fn write(&self, writer: &mut impl Write) -> Result<()> {
        VarInt(self.discriminant()).write(writer)
    }
}

/// Boss bar overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BossBarOverlay {
    /// Continuous progress bar.
    Progress,
    /// Six notches.
    Notched6,
    /// Ten notches.
    Notched10,
    /// Twelve notches.
    Notched12,
    /// Twenty notches.
    Notched20,
}

impl BossBarOverlay {
    const fn discriminant(self) -> i32 {
        match self {
            Self::Progress => 0,
            Self::Notched6 => 1,
            Self::Notched10 => 2,
            Self::Notched12 => 3,
            Self::Notched20 => 4,
        }
    }

    /// Returns the vanilla serialized name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Progress => "progress",
            Self::Notched6 => "notched_6",
            Self::Notched10 => "notched_10",
            Self::Notched12 => "notched_12",
            Self::Notched20 => "notched_20",
        }
    }

    /// Parses a vanilla boss bar overlay name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "progress" => Self::Progress,
            "notched_6" => Self::Notched6,
            "notched_10" => Self::Notched10,
            "notched_12" => Self::Notched12,
            "notched_20" => Self::Notched20,
            _ => return None,
        })
    }
}

impl WriteTo for BossBarOverlay {
    fn write(&self, writer: &mut impl Write) -> Result<()> {
        VarInt(self.discriminant()).write(writer)
    }
}

/// Boss bar screen effect properties.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BossBarProperties {
    /// Darken the sky.
    pub darken_screen: bool,
    /// Play boss music.
    pub play_music: bool,
    /// Create world fog.
    pub create_world_fog: bool,
}

impl BossBarProperties {
    fn flags(self) -> u8 {
        u8::from(self.darken_screen)
            | (u8::from(self.play_music) << 1)
            | (u8::from(self.create_world_fog) << 2)
    }
}

impl WriteTo for BossBarProperties {
    fn write(&self, writer: &mut impl Write) -> Result<()> {
        self.flags().write(writer)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use steel_utils::serial::{ReadFrom, WriteTo};

    use super::*;

    #[test]
    fn remove_packet_writes_uuid_and_remove_operation() {
        let id = Uuid::from_u128(0x1234);
        let mut bytes = Vec::new();

        CBossEvent::remove(id)
            .write(&mut bytes)
            .expect("boss event packet should write");

        let mut cursor = Cursor::new(bytes.as_slice());
        assert_eq!(Uuid::read(&mut cursor).expect("uuid should read"), id);
        assert_eq!(
            VarInt::read(&mut cursor).expect("operation should read").0,
            1
        );
        assert_eq!(cursor.position() as usize, bytes.len());
    }

    #[test]
    fn properties_flags_match_vanilla_bits() {
        let properties = BossBarProperties {
            darken_screen: true,
            play_music: false,
            create_world_fog: true,
        };

        assert_eq!(properties.flags(), 0b101);
    }
}
