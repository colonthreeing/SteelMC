use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tokio::io;
use uuid::Uuid;

use crate::permission::{
    PermissionEntry, PermissionMetadataEntry, PermissionMetadataExpression, PermissionMetadataSet,
    PermissionMetadataValue, PermissionRuleExpression, PermissionSet, PermissionState,
};

/// Server-wide player permission data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayerPermissionData {
    /// Assigned permission groups.
    pub groups: Vec<String>,
    /// Player-level permission overrides.
    pub permissions: PermissionSet,
    /// Player-level permission metadata overrides.
    pub metadata: PermissionMetadataSet,
}

impl PlayerPermissionData {
    /// Returns whether this player has any persisted permission state.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
            && self.permissions.entries().is_empty()
            && self.metadata.entries().is_empty()
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub(super) struct PlayerPermissionsFile {
    pub(super) players: BTreeMap<String, PlayerPermissionEntryFile>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub(super) struct PlayerPermissionEntryFile {
    pub(super) groups: Vec<String>,
    pub(super) allow: Vec<String>,
    pub(super) deny: Vec<String>,
    pub(super) metadata: Vec<PlayerPermissionMetadataEntryFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct PlayerPermissionMetadataEntryFile {
    pub(super) key: String,
    pub(super) value: PermissionMetadataValue,
}

impl PlayerPermissionsFile {
    pub(super) fn validate(&self) -> io::Result<()> {
        for (uuid, entry) in &self.players {
            let uuid = Uuid::parse_str(uuid).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid player permission UUID '{uuid}': {error}"),
                )
            })?;
            entry.validate(uuid)?;
        }
        Ok(())
    }

    pub(super) fn into_player_permission_data(
        self,
    ) -> io::Result<Vec<(Uuid, PlayerPermissionData)>> {
        let mut entries = Vec::with_capacity(self.players.len());
        for (uuid, entry) in self.players {
            let uuid = Uuid::parse_str(&uuid).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid player permission UUID '{uuid}': {error}"),
                )
            })?;
            entries.push((uuid, entry.into_player_permission_data(uuid)?));
        }
        Ok(entries)
    }
}

impl PlayerPermissionEntryFile {
    fn validate(&self, uuid: Uuid) -> io::Result<()> {
        for permission in &self.allow {
            parse_player_permission_expression(uuid, permission, "allow")?;
        }
        for permission in &self.deny {
            parse_player_permission_expression(uuid, permission, "deny")?;
        }
        for entry in &self.metadata {
            parse_player_metadata_expression(uuid, &entry.key)?;
        }
        Ok(())
    }

    fn from_player_permission_data(data: &PlayerPermissionData) -> Self {
        let mut allow = Vec::new();
        let mut deny = Vec::new();
        for entry in data.permissions.entries() {
            let expression =
                PermissionRuleExpression::new(entry.key().clone(), entry.context().clone())
                    .to_string();
            match entry.state() {
                PermissionState::Allow => allow.push(expression),
                PermissionState::Deny => deny.push(expression),
            }
        }

        Self {
            groups: data.groups.clone(),
            allow,
            deny,
            metadata: data
                .metadata
                .entries()
                .iter()
                .map(|entry| PlayerPermissionMetadataEntryFile {
                    key: PermissionMetadataExpression::new(
                        entry.key().clone(),
                        entry.context().clone(),
                    )
                    .to_string(),
                    value: entry.value().clone(),
                })
                .collect(),
        }
    }

    pub(super) fn into_player_permission_data(
        self,
        uuid: Uuid,
    ) -> io::Result<PlayerPermissionData> {
        let mut permissions = PermissionSet::new();
        for permission in self.allow {
            let expression = parse_player_permission_expression(uuid, &permission, "allow")?;
            let (key, context) = expression.into_parts();
            permissions.push(PermissionEntry::allow_with_context(key, context));
        }
        for permission in self.deny {
            let expression = parse_player_permission_expression(uuid, &permission, "deny")?;
            let (key, context) = expression.into_parts();
            permissions.push(PermissionEntry::deny_with_context(key, context));
        }

        let mut metadata = PermissionMetadataSet::new();
        for entry in self.metadata {
            let expression = parse_player_metadata_expression(uuid, &entry.key)?;
            let (key, context) = expression.into_parts();
            metadata.push(PermissionMetadataEntry::new_with_context(
                key,
                context,
                entry.value,
            ));
        }

        Ok(PlayerPermissionData {
            groups: self.groups,
            permissions,
            metadata,
        })
    }
}

fn parse_player_permission_expression(
    uuid: Uuid,
    permission: &str,
    state: &str,
) -> io::Result<PermissionRuleExpression> {
    PermissionRuleExpression::parse(permission).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid {state} permission expression for {uuid}: {error}"),
        )
    })
}

fn parse_player_metadata_expression(
    uuid: Uuid,
    key: &str,
) -> io::Result<PermissionMetadataExpression> {
    PermissionMetadataExpression::parse(key).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid permission metadata expression for {uuid}: {error}"),
        )
    })
}

pub(super) fn set_player_permission_entry(
    file: &mut PlayerPermissionsFile,
    uuid: Uuid,
    data: &PlayerPermissionData,
) {
    if data.is_empty() {
        file.players.remove(&uuid.to_string());
        return;
    }

    file.players.insert(
        uuid.to_string(),
        PlayerPermissionEntryFile::from_player_permission_data(data),
    );
}

pub(super) fn serialize_player_permissions_file(
    file: &PlayerPermissionsFile,
) -> Result<String, toml::ser::Error> {
    let mut output = String::new();
    if file.players.is_empty() {
        output.push_str("players = {}\n");
        return Ok(output);
    }

    for (uuid, entry) in &file.players {
        output.push_str("[players.");
        output.push_str(&toml_value(uuid)?);
        output.push_str("]\n");
        push_player_permission_entry(&mut output, entry)?;
        output.push('\n');
    }
    Ok(output)
}

fn push_player_permission_entry(
    output: &mut String,
    entry: &PlayerPermissionEntryFile,
) -> Result<(), toml::ser::Error> {
    push_string_array_field(output, "groups", &entry.groups)?;
    push_string_array_field(output, "allow", &entry.allow)?;
    push_string_array_field(output, "deny", &entry.deny)?;
    push_permission_metadata_entries(output, &entry.metadata)?;
    Ok(())
}

fn push_string_array_field(
    output: &mut String,
    key: &str,
    values: &[String],
) -> Result<(), toml::ser::Error> {
    if values.is_empty() {
        output.push_str(key);
        output.push_str(" = []\n");
        return Ok(());
    }

    output.push_str(key);
    output.push_str(" = [\n");
    for value in values {
        output.push_str("    ");
        output.push_str(&toml_value(value)?);
        output.push_str(",\n");
    }
    output.push_str("]\n");
    Ok(())
}

fn push_permission_metadata_entries(
    output: &mut String,
    metadata: &[PlayerPermissionMetadataEntryFile],
) -> Result<(), toml::ser::Error> {
    if metadata.is_empty() {
        output.push_str("metadata = []\n");
        return Ok(());
    }

    output.push_str("metadata = [\n");
    for entry in metadata {
        output.push_str("    { key = ");
        output.push_str(&toml_value(&entry.key)?);
        output.push_str(", value = ");
        output.push_str(&permission_metadata_value_toml(&entry.value)?);
        output.push_str(" },\n");
    }
    output.push_str("]\n");
    Ok(())
}

fn toml_value<T: Serialize + ?Sized>(value: &T) -> Result<String, toml::ser::Error> {
    #[derive(Serialize)]
    struct Field<'a, T: Serialize + ?Sized> {
        value: &'a T,
    }

    let serialized = toml::to_string(&Field { value })?;
    Ok(serialized
        .trim_end()
        .strip_prefix("value = ")
        .unwrap_or(serialized.trim_end())
        .to_owned())
}

fn permission_metadata_value_toml(
    value: &PermissionMetadataValue,
) -> Result<String, toml::ser::Error> {
    match value {
        PermissionMetadataValue::Bool(value) => toml_value(value),
        PermissionMetadataValue::Integer(value) => toml_value(value),
        PermissionMetadataValue::String(value) => toml_value(value),
    }
}
