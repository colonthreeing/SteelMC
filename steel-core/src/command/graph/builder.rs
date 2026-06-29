use std::sync::Arc;

use super::node::{CommandNode, CommandNodeKind, CommandRedirect};
use super::{
    CommandArgumentParser, CommandExecutor, CommandGraphError, CommandPermissionArgument,
    CommandRedirectTarget, CommandResult, DynamicPermission, ParsedArguments,
    UnresolvedDynamicPermission,
};
use crate::command::{context::CommandContext, error::CommandError, requirement::Requirement};
use crate::permission::{
    PermissionCatalog, PermissionCatalogSource, PermissionExpr, PermissionKey, PermissionSegment,
};

/// Builds a command graph node.
#[derive(Clone)]
pub struct CommandNodeBuilder {
    kind: CommandNodeKind,
    requirement: Requirement,
    children: Vec<CommandNodeBuilder>,
    executor: Option<CommandExecutor>,
    redirect: Option<CommandRedirect>,
    derived_subcommand_permission: Option<DerivedSubcommandPermissionMode>,
    dynamic_permissions: Vec<UnresolvedDynamicPermission>,
    resolved_dynamic_permissions: Vec<DynamicPermission>,
    catalog_permissions: Vec<PermissionKey>,
}

#[derive(Clone, Copy)]
enum DerivedSubcommandPermissionMode {
    AlternativeToRoot,
    AdditionalToRoot,
}

impl CommandNodeBuilder {
    /// Adds a child node.
    #[must_use]
    pub fn then(mut self, child: Self) -> Self {
        self.children.push(child);
        self
    }

    /// Adds multiple child nodes in order.
    #[must_use]
    pub fn then_all(mut self, children: impl IntoIterator<Item = Self>) -> Self {
        self.children.extend(children);
        self
    }

    /// Adds a requirement to this node.
    #[must_use]
    pub fn requires(mut self, requirement: Requirement) -> Self {
        collect_requirement_catalog_permissions(&requirement, &mut self.catalog_permissions);
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
    /// The command root permission also grants this node, while this derived
    /// permission grants only this node path.
    #[must_use]
    pub const fn requires_subcommand_permission(mut self) -> Self {
        self.derived_subcommand_permission =
            Some(DerivedSubcommandPermissionMode::AlternativeToRoot);
        self
    }

    /// Adds a derived subcommand permission that must be held in addition to
    /// the command root permission.
    ///
    /// Use this for administrative subcommands where the root command grants
    /// access to the command family but not to every sensitive action.
    #[must_use]
    pub const fn requires_additional_subcommand_permission(mut self) -> Self {
        self.derived_subcommand_permission =
            Some(DerivedSubcommandPermissionMode::AdditionalToRoot);
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
        self.validate_client_parser_contract()?;
        if !self.dynamic_permissions.is_empty() {
            return Err(CommandGraphError::UnresolvedDynamicPermission {
                name: self.kind.display_name().to_owned(),
            });
        }
        if self.derived_subcommand_permission.is_some() {
            return Err(CommandGraphError::UnresolvedDerivedPermission {
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

    fn validate_client_parser_contract(&self) -> Result<(), CommandGraphError> {
        let CommandNodeKind::Argument { name, parser } = &self.kind else {
            return Ok(());
        };

        if parser.is_terminal_argument() && (!self.children.is_empty() || self.redirect.is_some()) {
            return Err(CommandGraphError::TerminalArgumentMustBeLeaf { name: name.clone() });
        }

        Ok(())
    }

    pub(crate) fn resolve_subcommand_permissions(
        self,
        root_permission: &PermissionKey,
        catalog: &mut PermissionCatalog,
    ) -> Result<Self, CommandGraphError> {
        let mut alternate_root_permissions = Vec::new();
        let resolved = self.resolve_subcommand_permissions_inner(
            root_permission,
            root_permission,
            true,
            Vec::new(),
            catalog,
            &mut alternate_root_permissions,
        )?;
        Ok(resolved.with_root_access_requirement(root_permission, alternate_root_permissions))
    }

    fn resolve_subcommand_permissions_inner(
        self,
        root_permission: &PermissionKey,
        parent_permission: &PermissionKey,
        is_root: bool,
        available_arguments: Vec<AvailableArgument>,
        catalog: &mut PermissionCatalog,
        alternate_root_permissions: &mut Vec<PermissionKey>,
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
            catalog_permissions,
        } = self;

        let child_permission;
        let mut available_arguments = available_arguments;
        let current_permission = match &kind {
            CommandNodeKind::Literal(_) if is_root => parent_permission,
            CommandNodeKind::Literal(name) => {
                let segment = PermissionSegment::parse(name.as_str())?;
                child_permission = parent_permission.child(&segment)?;
                &child_permission
            }
            CommandNodeKind::Argument { name, parser } => {
                available_arguments.push(AvailableArgument {
                    name: name.clone(),
                    parsed_type: parser.parsed_type(),
                });
                parent_permission
            }
        };

        if let Some(permission_mode) = derived_subcommand_permission {
            if is_root || !matches!(kind, CommandNodeKind::Literal(_)) {
                return Err(CommandGraphError::DerivedPermissionRequiresLiteral {
                    name: kind.display_name().to_owned(),
                });
            }
            let current_permission = current_permission.to_owned();
            let requirement_permission = match permission_mode {
                DerivedSubcommandPermissionMode::AlternativeToRoot => {
                    push_unique_permission(alternate_root_permissions, current_permission.clone());
                    PermissionExpr::scoped_key(root_permission.clone(), current_permission.clone())
                }
                DerivedSubcommandPermissionMode::AdditionalToRoot => {
                    PermissionExpr::key(root_permission.clone())
                        & PermissionExpr::key(current_permission.clone())
                }
            };
            requirement = requirement.and(Requirement::Permission(requirement_permission));
            catalog.insert(current_permission, PermissionCatalogSource::Command);
        }

        let mut resolved_dynamic_permissions = resolved_dynamic_permissions;
        for permission in &catalog_permissions {
            catalog.insert(permission.clone(), PermissionCatalogSource::Command);
        }
        for permission in &dynamic_permissions {
            validate_dynamic_permission_argument(
                kind.display_name(),
                permission,
                &available_arguments,
            )?;
            for catalog_permission in register_dynamic_permission_catalog_entries(
                current_permission,
                permission,
                catalog,
            )? {
                push_unique_permission(alternate_root_permissions, catalog_permission);
            }
        }
        resolved_dynamic_permissions.extend(
            dynamic_permissions
                .into_iter()
                .map(|permission| permission.resolve(root_permission, current_permission)),
        );

        Ok(Self {
            kind,
            requirement,
            children: children
                .into_iter()
                .map(|child| {
                    child.resolve_subcommand_permissions_inner(
                        root_permission,
                        current_permission,
                        false,
                        available_arguments.clone(),
                        catalog,
                        alternate_root_permissions,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
            executor,
            redirect,
            derived_subcommand_permission: None,
            dynamic_permissions: Vec::new(),
            resolved_dynamic_permissions,
            catalog_permissions,
        })
    }

    fn with_root_access_requirement(
        self,
        root_permission: &PermissionKey,
        alternate_permissions: Vec<PermissionKey>,
    ) -> Self {
        let permission = alternate_permissions.into_iter().fold(
            PermissionExpr::key(root_permission.clone()),
            |permission, alternative| permission | PermissionExpr::key(alternative),
        );
        self.requires(Requirement::Permission(permission))
    }
}

#[derive(Clone)]
struct AvailableArgument {
    name: String,
    parsed_type: &'static str,
}

fn validate_dynamic_permission_argument(
    node_name: &str,
    permission: &UnresolvedDynamicPermission,
    available_arguments: &[AvailableArgument],
) -> Result<(), CommandGraphError> {
    let Some(argument) = available_arguments
        .iter()
        .rev()
        .find(|argument| argument.name == permission.argument_name)
    else {
        return Err(CommandGraphError::MissingDynamicPermissionArgument {
            node: node_name.to_owned(),
            argument: permission.argument_name.clone(),
        });
    };

    if argument.parsed_type != permission.expected_type {
        return Err(CommandGraphError::WrongDynamicPermissionArgumentType {
            node: node_name.to_owned(),
            argument: permission.argument_name.clone(),
            expected: permission.expected_type,
            actual: argument.parsed_type,
        });
    }

    Ok(())
}

fn register_dynamic_permission_catalog_entries(
    base_permission: &PermissionKey,
    permission: &UnresolvedDynamicPermission,
    catalog: &mut PermissionCatalog,
) -> Result<Vec<PermissionKey>, CommandGraphError> {
    let mut registered = Vec::new();
    for segment in permission.catalog_segments {
        let segment = PermissionSegment::parse(*segment)?;
        let key = base_permission.child(&segment)?;
        catalog.insert(key.clone(), PermissionCatalogSource::Command);
        registered.push(key);
    }
    Ok(registered)
}

fn push_unique_permission(permissions: &mut Vec<PermissionKey>, permission: PermissionKey) {
    if permissions.iter().any(|existing| existing == &permission) {
        return;
    }
    permissions.push(permission);
}

fn collect_requirement_catalog_permissions(
    requirement: &Requirement,
    catalog_permissions: &mut Vec<PermissionKey>,
) {
    match requirement {
        Requirement::Permission(permission) => {
            collect_permission_expr_keys(permission, catalog_permissions);
        }
        Requirement::All(requirements) | Requirement::Any(requirements) => {
            for requirement in requirements {
                collect_requirement_catalog_permissions(requirement, catalog_permissions);
            }
        }
        Requirement::Always | Requirement::Player | Requirement::Console => {}
    }
}

fn collect_permission_expr_keys(
    permission: &PermissionExpr,
    catalog_permissions: &mut Vec<PermissionKey>,
) {
    match permission {
        PermissionExpr::Key(key) => {
            if !catalog_permissions.iter().any(|existing| existing == key) {
                catalog_permissions.push(key.clone());
            }
        }
        PermissionExpr::ScopedKey { parent, key } => {
            if !catalog_permissions
                .iter()
                .any(|existing| existing == parent)
            {
                catalog_permissions.push(parent.clone());
            }
            if !catalog_permissions.iter().any(|existing| existing == key) {
                catalog_permissions.push(key.clone());
            }
        }
        PermissionExpr::All(children) | PermissionExpr::Any(children) => {
            for child in children {
                collect_permission_expr_keys(child, catalog_permissions);
            }
        }
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
        derived_subcommand_permission: None,
        dynamic_permissions: Vec::new(),
        resolved_dynamic_permissions: Vec::new(),
        catalog_permissions: Vec::new(),
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
        derived_subcommand_permission: None,
        dynamic_permissions: Vec::new(),
        resolved_dynamic_permissions: Vec::new(),
        catalog_permissions: Vec::new(),
    }
}
