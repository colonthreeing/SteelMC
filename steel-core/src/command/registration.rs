use std::{error::Error, fmt};

use crate::command::graph::{CommandGraphError, CommandNodeBuilder, validate_command_node_name};
use crate::permission::{PermissionExpr, PermissionKey, PermissionKeyError, PermissionSegment};

pub(crate) struct CommandRegistration {
    pub(super) root: CommandNodeBuilder,
    namespace: PermissionSegment,
    permission: CommandPermissionMode,
    pub(super) aliases: Vec<String>,
}

impl CommandRegistration {
    pub(crate) const fn new(root: CommandNodeBuilder, namespace: PermissionSegment) -> Self {
        Self {
            root,
            namespace,
            permission: CommandPermissionMode::Auto,
            aliases: Vec::new(),
        }
    }

    pub(crate) fn minecraft(root: CommandNodeBuilder) -> Result<Self, CommandRegistrationError> {
        Ok(Self::new(root, PermissionSegment::parse("minecraft")?))
    }

    pub(crate) fn steel(root: CommandNodeBuilder) -> Result<Self, CommandRegistrationError> {
        Ok(Self::new(root, PermissionSegment::parse("steel")?))
    }

    pub(crate) fn public(mut self) -> Self {
        self.permission = CommandPermissionMode::Public;
        self
    }

    pub(crate) fn permission(mut self, permission: PermissionKey) -> Self {
        self.permission = CommandPermissionMode::Override(permission);
        self
    }

    pub(crate) fn permission_base(self, command: &str) -> Result<Self, CommandRegistrationError> {
        let permission = command_permission_key(&self.namespace, command)?;
        Ok(self.permission(permission))
    }

    pub(crate) fn alias(mut self, alias: &str) -> Result<Self, CommandRegistrationError> {
        validate_command_node_name(alias).map_err(|source| {
            CommandGraphError::InvalidLiteralName {
                name: alias.to_owned(),
                source,
            }
        })?;
        self.aliases.push(alias.to_owned());
        Ok(self)
    }

    pub(super) fn resolved_permission_base(
        &self,
    ) -> Result<Option<PermissionKey>, CommandRegistrationError> {
        match &self.permission {
            CommandPermissionMode::Public => Ok(None),
            CommandPermissionMode::Auto => {
                let command_name = self
                    .root
                    .literal_name()
                    .ok_or(CommandRegistrationError::RootMustBeLiteral)?;
                Ok(Some(command_permission_key(&self.namespace, command_name)?))
            }
            CommandPermissionMode::Override(permission) => Ok(Some(permission.clone())),
        }
    }
}

/// Static registration metadata for one built-in command module.
#[derive(Clone, Copy)]
pub(crate) struct CommandRegistrationSpec {
    namespace: CommandRegistrationNamespace,
    permission: CommandRegistrationSpecPermission,
    aliases: &'static [&'static str],
}

impl CommandRegistrationSpec {
    pub(crate) const fn minecraft() -> Self {
        Self::new(CommandRegistrationNamespace::Minecraft)
    }

    pub(crate) const fn steel() -> Self {
        Self::new(CommandRegistrationNamespace::Steel)
    }

    const fn new(namespace: CommandRegistrationNamespace) -> Self {
        Self {
            namespace,
            permission: CommandRegistrationSpecPermission::Auto,
            aliases: &[],
        }
    }

    pub(crate) const fn public(mut self) -> Self {
        self.permission = CommandRegistrationSpecPermission::Public;
        self
    }

    pub(crate) const fn permission_base(mut self, command: &'static str) -> Self {
        self.permission = CommandRegistrationSpecPermission::PermissionBase(command);
        self
    }

    pub(crate) const fn aliases(mut self, aliases: &'static [&'static str]) -> Self {
        self.aliases = aliases;
        self
    }

    pub(super) fn register(
        self,
        root: CommandNodeBuilder,
    ) -> Result<CommandRegistration, CommandRegistrationError> {
        let registration = match self.namespace {
            CommandRegistrationNamespace::Minecraft => CommandRegistration::minecraft(root)?,
            CommandRegistrationNamespace::Steel => CommandRegistration::steel(root)?,
        };
        let mut registration = match self.permission {
            CommandRegistrationSpecPermission::Auto => registration,
            CommandRegistrationSpecPermission::Public => registration.public(),
            CommandRegistrationSpecPermission::PermissionBase(command) => {
                registration.permission_base(command)?
            }
        };
        for alias in self.aliases {
            registration = registration.alias(alias)?;
        }
        Ok(registration)
    }
}

#[derive(Clone, Copy)]
enum CommandRegistrationNamespace {
    Minecraft,
    Steel,
}

#[derive(Clone, Copy)]
enum CommandRegistrationSpecPermission {
    Auto,
    Public,
    PermissionBase(&'static str),
}

enum CommandPermissionMode {
    Auto,
    Public,
    Override(PermissionKey),
}

/// Invalid command registration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandRegistrationError {
    /// A command root was not a literal node.
    RootMustBeLiteral,
    /// A command or alias produced an invalid permission key.
    InvalidPermissionKey(PermissionKeyError),
    /// The command graph rejected a node registration.
    InvalidGraph(CommandGraphError),
    /// Command graph validation found diagnostics after registration.
    InvalidGraphValidation(crate::command::graph::CommandGraphValidation),
}

impl fmt::Display for CommandRegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RootMustBeLiteral => write!(f, "command root must be a literal node"),
            Self::InvalidPermissionKey(error) => write!(f, "{error}"),
            Self::InvalidGraph(error) => write!(f, "{error}"),
            Self::InvalidGraphValidation(validation) => {
                write!(f, "command graph validation failed")?;
                if let Some(ambiguity) = validation.ambiguities().first() {
                    write!(
                        f,
                        ": ambiguous child '{}' and sibling '{}' under '{}'",
                        ambiguity.child,
                        ambiguity.sibling,
                        ambiguity.parent_path.join(" ")
                    )?;
                }
                Ok(())
            }
        }
    }
}

impl Error for CommandRegistrationError {}

impl From<PermissionKeyError> for CommandRegistrationError {
    fn from(value: PermissionKeyError) -> Self {
        Self::InvalidPermissionKey(value)
    }
}

impl From<CommandGraphError> for CommandRegistrationError {
    fn from(value: CommandGraphError) -> Self {
        Self::InvalidGraph(value)
    }
}

pub(super) fn command_permission_key(
    namespace: &PermissionSegment,
    command: &str,
) -> Result<PermissionKey, PermissionKeyError> {
    PermissionKey::from_segments([
        namespace.clone(),
        PermissionSegment::parse("command")?,
        PermissionSegment::parse(command)?,
    ])
}

pub(crate) fn minecraft_command_permission_key(
    command: &str,
) -> Result<PermissionKey, PermissionKeyError> {
    command_permission_key(&PermissionSegment::parse("minecraft")?, command)
}

pub(crate) const ENTITY_SELECTOR_PERMISSION_KEY: &str = "minecraft.selector";
pub(crate) const ENTITY_SELECTOR_ADVANCED_PERMISSION_KEY: &str = "minecraft.selector.advanced";

pub(crate) fn entity_selector_permission_key() -> Result<PermissionKey, PermissionKeyError> {
    PermissionKey::parse(ENTITY_SELECTOR_PERMISSION_KEY)
}

pub(crate) fn entity_selector_advanced_permission_key() -> Result<PermissionKey, PermissionKeyError>
{
    PermissionKey::parse(ENTITY_SELECTOR_ADVANCED_PERMISSION_KEY)
}

pub(crate) fn entity_selector_permission_expr() -> Result<PermissionExpr, PermissionKeyError> {
    Ok(PermissionExpr::key(entity_selector_permission_key()?))
}

pub(crate) fn entity_selector_advanced_permission_expr()
-> Result<PermissionExpr, PermissionKeyError> {
    Ok(PermissionExpr::key(
        entity_selector_advanced_permission_key()?,
    ))
}
