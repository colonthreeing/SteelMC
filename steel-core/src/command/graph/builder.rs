use std::sync::Arc;

use super::node::{CommandNode, CommandNodeKind, CommandRedirect};
use super::{
    CommandArgumentParser, CommandExecutor, CommandGraphError, CommandPermissionArgument,
    CommandRedirectTarget, CommandResult, DynamicPermission, ParsedArguments,
    UnresolvedDynamicPermission,
};
use crate::command::{context::CommandContext, error::CommandError, requirement::Requirement};
use crate::permission::{PermissionExpr, PermissionKey, PermissionSegment};

/// Builds a command graph node.
#[derive(Clone)]
pub struct CommandNodeBuilder {
    kind: CommandNodeKind,
    requirement: Requirement,
    children: Vec<CommandNodeBuilder>,
    executor: Option<CommandExecutor>,
    redirect: Option<CommandRedirect>,
    derived_subcommand_permission: bool,
    dynamic_permissions: Vec<UnresolvedDynamicPermission>,
    resolved_dynamic_permissions: Vec<DynamicPermission>,
}

impl CommandNodeBuilder {
    /// Adds a child node.
    #[must_use]
    pub fn then(mut self, child: Self) -> Self {
        self.children.push(child);
        self
    }

    /// Adds a requirement to this node.
    #[must_use]
    pub fn requires(mut self, requirement: Requirement) -> Self {
        self.requirement = self.requirement.and(requirement);
        self
    }

    /// Adds a permission requirement to this node.
    #[must_use]
    pub fn requires_permission(self, permission: PermissionKey) -> Self {
        self.requires_permission_expr(PermissionExpr::key(permission))
    }

    /// Adds a permission expression requirement to this node.
    #[must_use]
    pub fn requires_permission_expr(self, permission: PermissionExpr) -> Self {
        self.requires(Requirement::Permission(permission))
    }

    /// Adds a permission derived from the command's literal subcommand path.
    ///
    /// This must be used on a non-root literal node. The command registration
    /// resolves it by appending this literal path to the root command permission.
    /// For example, `tick freeze` derives `minecraft.command.tick.freeze`.
    /// Use [`Self::requires_permission`] when a subcommand needs a custom key.
    #[must_use]
    pub const fn requires_subcommand_permission(mut self) -> Self {
        self.derived_subcommand_permission = true;
        self
    }

    /// Adds a dynamic permission derived from a parsed argument value.
    ///
    /// The command registration resolves the base permission from this node's
    /// literal path. At execution time, `argument_name` is read as `T`, converted
    /// into one permission segment, and appended to that base.
    #[must_use]
    pub fn requires_argument_permission<T>(mut self, argument_name: impl Into<String>) -> Self
    where
        T: CommandPermissionArgument + 'static,
    {
        self.dynamic_permissions
            .push(UnresolvedDynamicPermission::argument::<T>(
                argument_name.into(),
            ));
        self
    }

    /// Returns this literal node's name.
    #[must_use]
    pub fn literal_name(&self) -> Option<&str> {
        match &self.kind {
            CommandNodeKind::Literal(name) => Some(name),
            CommandNodeKind::Argument { .. } => None,
        }
    }

    /// Returns this node with a different literal name.
    #[must_use]
    pub fn with_literal_name(mut self, name: impl Into<String>) -> Option<Self> {
        match &mut self.kind {
            CommandNodeKind::Literal(literal) => {
                *literal = name.into();
                Some(self)
            }
            CommandNodeKind::Argument { .. } => None,
        }
    }

    /// Marks this node executable.
    #[must_use]
    pub fn executes(
        mut self,
        executor: impl Fn(&mut CommandContext, &ParsedArguments) -> Result<CommandResult, CommandError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.executor = Some(Arc::new(executor));
        self
    }

    /// Redirects from this node after running `executor`.
    #[must_use]
    pub fn redirects(
        mut self,
        target: CommandRedirectTarget,
        executor: impl Fn(&mut CommandContext, &ParsedArguments) -> Result<CommandResult, CommandError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.redirect = Some(CommandRedirect {
            target,
            executor: Arc::new(executor),
        });
        self
    }

    pub(super) fn build(self) -> Result<CommandNode, CommandGraphError> {
        self.kind.validate()?;
        if !self.dynamic_permissions.is_empty() {
            return Err(CommandGraphError::UnresolvedDynamicPermission {
                name: self.kind.display_name().to_owned(),
            });
        }
        Ok(CommandNode {
            kind: self.kind,
            requirement: self.requirement,
            children: self
                .children
                .into_iter()
                .map(Self::build)
                .collect::<Result<Vec<_>, _>>()?,
            executor: self.executor,
            redirect: self.redirect,
            dynamic_permissions: self.resolved_dynamic_permissions,
        })
    }

    pub(crate) fn resolve_subcommand_permissions(
        self,
        root_permission: &PermissionKey,
    ) -> Result<Self, CommandGraphError> {
        self.resolve_subcommand_permissions_inner(root_permission, true)
    }

    fn resolve_subcommand_permissions_inner(
        self,
        parent_permission: &PermissionKey,
        is_root: bool,
    ) -> Result<Self, CommandGraphError> {
        let Self {
            kind,
            mut requirement,
            children,
            executor,
            redirect,
            derived_subcommand_permission,
            dynamic_permissions,
            resolved_dynamic_permissions,
        } = self;

        let child_permission;
        let current_permission = match &kind {
            CommandNodeKind::Literal(_) if is_root => parent_permission,
            CommandNodeKind::Literal(name) => {
                let segment = PermissionSegment::parse(name.as_str())?;
                child_permission = parent_permission.child(&segment)?;
                &child_permission
            }
            CommandNodeKind::Argument { .. } => parent_permission,
        };

        if derived_subcommand_permission {
            if is_root || !matches!(kind, CommandNodeKind::Literal(_)) {
                return Err(CommandGraphError::DerivedPermissionRequiresLiteral {
                    name: kind.display_name().to_owned(),
                });
            }
            requirement = requirement.and(Requirement::Permission(PermissionExpr::key(
                current_permission.to_owned(),
            )));
        }

        let mut resolved_dynamic_permissions = resolved_dynamic_permissions;
        resolved_dynamic_permissions.extend(
            dynamic_permissions
                .into_iter()
                .map(|permission| permission.resolve(current_permission)),
        );

        Ok(Self {
            kind,
            requirement,
            children: children
                .into_iter()
                .map(|child| child.resolve_subcommand_permissions_inner(current_permission, false))
                .collect::<Result<Vec<_>, _>>()?,
            executor,
            redirect,
            derived_subcommand_permission: false,
            dynamic_permissions: Vec::new(),
            resolved_dynamic_permissions,
        })
    }
}

/// Creates a literal node builder.
#[must_use]
pub fn literal(name: impl Into<String>) -> CommandNodeBuilder {
    CommandNodeBuilder {
        kind: CommandNodeKind::Literal(name.into()),
        requirement: Requirement::Always,
        children: Vec::new(),
        executor: None,
        redirect: None,
        derived_subcommand_permission: false,
        dynamic_permissions: Vec::new(),
        resolved_dynamic_permissions: Vec::new(),
    }
}

/// Creates an argument node builder.
#[must_use]
pub fn argument(
    name: impl Into<String>,
    parser: impl CommandArgumentParser + 'static,
) -> CommandNodeBuilder {
    CommandNodeBuilder {
        kind: CommandNodeKind::Argument {
            name: name.into(),
            parser: Arc::new(parser),
        },
        requirement: Requirement::Always,
        children: Vec::new(),
        executor: None,
        redirect: None,
        derived_subcommand_permission: false,
        dynamic_permissions: Vec::new(),
        resolved_dynamic_permissions: Vec::new(),
    }
}
