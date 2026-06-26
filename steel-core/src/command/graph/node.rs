use std::sync::Arc;

use steel_protocol::packets::game::{CommandNode as ProtocolCommandNode, CommandNodeInfo};

use super::{
    CommandArgumentParser, CommandExecutor, CommandGraphError, CommandRedirectTarget,
    DynamicPermission, validate_command_node_name,
};
use crate::command::requirement::{
    CommandSourceKind, PermissionExpr, Requirement, RequirementContext,
};

struct NoPermissionContext {
    source_kind: CommandSourceKind,
}

impl RequirementContext for NoPermissionContext {
    fn source_kind(&self) -> CommandSourceKind {
        self.source_kind
    }

    fn has_permission(&self, _permission: &PermissionExpr) -> bool {
        false
    }
}

#[derive(Clone)]
pub(super) struct CommandRedirect {
    pub(super) target: CommandRedirectTarget,
    pub(super) executor: CommandExecutor,
}

#[derive(Clone)]
pub(super) struct CommandNode {
    pub(super) kind: CommandNodeKind,
    pub(super) requirement: Requirement,
    pub(super) children: Vec<CommandNode>,
    pub(super) executor: Option<CommandExecutor>,
    pub(super) redirect: Option<CommandRedirect>,
    pub(super) dynamic_permissions: Vec<DynamicPermission>,
}

impl CommandNode {
    pub(super) fn display_name(&self) -> &str {
        match &self.kind {
            CommandNodeKind::Literal(name) | CommandNodeKind::Argument { name, .. } => name,
        }
    }

    pub(super) fn usage(
        &self,
        buffer: &mut Vec<ProtocolCommandNode>,
        siblings: &mut Vec<i32>,
        context: &dyn RequirementContext,
        current_root_index: Option<i32>,
    ) {
        if !self.requirement.allows(context) {
            return;
        }

        let node_index = buffer.len();
        buffer.push(ProtocolCommandNode::new_root());
        siblings.push(node_index as i32);
        let current_root_index = current_root_index.unwrap_or(node_index as i32);

        let mut children = Vec::new();
        if self.redirect.is_none() {
            for child in &self.children {
                child.usage(buffer, &mut children, context, Some(current_root_index));
            }
        }

        let mut info = self.redirect.as_ref().map_or_else(
            || CommandNodeInfo::new(children),
            |redirect| {
                CommandNodeInfo::new_redirect(match redirect.target {
                    CommandRedirectTarget::Current => current_root_index,
                    CommandRedirectTarget::All => 0,
                })
            },
        );
        if self.redirect.is_none() && self.executor.is_some() {
            info = info.chain(CommandNodeInfo::new_executable());
        }
        if self.is_restricted(context) {
            info = info.restricted();
        }

        buffer[node_index] = match &self.kind {
            CommandNodeKind::Literal(name) => ProtocolCommandNode::new_literal(info, name.clone()),
            CommandNodeKind::Argument { name, parser } => {
                ProtocolCommandNode::new_argument(info, name.clone(), parser.usage())
            }
        };
    }

    fn is_restricted(&self, context: &dyn RequirementContext) -> bool {
        let no_permission_context = NoPermissionContext {
            source_kind: context.source_kind(),
        };
        !self.requirement.allows(&no_permission_context)
    }

    fn can_merge_with(&self, other: &Self) -> bool {
        self.requirement == other.requirement
            && self.dynamic_permissions.is_empty()
            && other.dynamic_permissions.is_empty()
            && self.redirect.is_none()
            && other.redirect.is_none()
            && !(self.executor.is_some() && other.executor.is_some())
            && matches!(
                (&self.kind, &other.kind),
                (CommandNodeKind::Literal(left), CommandNodeKind::Literal(right)) if left == right
            )
    }

    fn same_literal_name(&self, other: &Self) -> Option<&str> {
        match (&self.kind, &other.kind) {
            (CommandNodeKind::Literal(left), CommandNodeKind::Literal(right)) if left == right => {
                Some(left)
            }
            _ => None,
        }
    }

    fn merge(&mut self, other: Self) -> Result<(), CommandGraphError> {
        if self.executor.is_none() {
            self.executor = other.executor;
        }

        for child in other.children {
            merge_or_push_node(&mut self.children, child)?;
        }

        Ok(())
    }
}

#[derive(Clone)]
pub(super) enum CommandNodeKind {
    Literal(String),
    Argument {
        name: String,
        parser: Arc<dyn CommandArgumentParser>,
    },
}

impl CommandNodeKind {
    pub(super) fn display_name(&self) -> &str {
        match self {
            Self::Literal(name) | Self::Argument { name, .. } => name,
        }
    }

    pub(super) fn validate(&self) -> Result<(), CommandGraphError> {
        match self {
            Self::Literal(name) => validate_command_node_name(name).map_err(|source| {
                CommandGraphError::InvalidLiteralName {
                    name: name.to_owned(),
                    source,
                }
            }),
            Self::Argument { name, .. } => validate_command_node_name(name).map_err(|source| {
                CommandGraphError::InvalidArgumentName {
                    name: name.to_owned(),
                    source,
                }
            }),
        }
    }
}

pub(super) fn merge_or_push_node(
    nodes: &mut Vec<CommandNode>,
    node: CommandNode,
) -> Result<(), CommandGraphError> {
    let Some(existing_index) = nodes
        .iter()
        .position(|existing| existing.same_literal_name(&node).is_some())
    else {
        nodes.push(node);
        return Ok(());
    };

    let existing = &mut nodes[existing_index];
    if !existing.can_merge_with(&node) {
        return Err(CommandGraphError::LiteralCollision {
            name: node.display_name().to_owned(),
        });
    }

    existing.merge(node)
}
