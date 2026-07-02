//! Group permission config editing helpers for the `steelperms` command.

use std::fmt;

use steel_utils::Identifier;

use crate::permission::{
    OP_GROUP, PermissionGroupConfig, PermissionGroupsConfig, PermissionKey,
    PermissionMetadataExpression, PermissionMetadataRuleConfig, PermissionRuleContext,
    PermissionRuleExpression, PermissionState, PermissionMetadataValue,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PermissionGroupEditError {
    AlreadyExists(String),
    Missing(String),
    Required(String),
    Default(String),
}

impl fmt::Display for PermissionGroupEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyExists(group) => write!(f, "permission group '{group}' already exists"),
            Self::Missing(group) => write!(f, "unknown permission group '{group}'"),
            Self::Required(group) => write!(f, "permission group '{group}' is required"),
            Self::Default(group) => {
                write!(f, "permission group '{group}' is still a default group")
            }
        }
    }
}

impl std::error::Error for PermissionGroupEditError {}

pub(super) fn create_group_config(
    config: &mut PermissionGroupsConfig,
    group: &str,
) -> Result<(), PermissionGroupEditError> {
    if config.groups.contains_key(group) {
        return Err(PermissionGroupEditError::AlreadyExists(group.to_owned()));
    }

    config
        .groups
        .insert(group.to_owned(), PermissionGroupConfig::default());
    Ok(())
}

pub(super) fn delete_group_config(
    config: &mut PermissionGroupsConfig,
    group: &str,
) -> Result<(), PermissionGroupEditError> {
    if group == OP_GROUP {
        return Err(PermissionGroupEditError::Required(group.to_owned()));
    }
    if !config.groups.contains_key(group) {
        return Err(PermissionGroupEditError::Missing(group.to_owned()));
    }
    if config.default_groups.iter().any(|default| default == group) {
        return Err(PermissionGroupEditError::Default(group.to_owned()));
    }

    config.groups.remove(group);
    Ok(())
}

pub(super) fn add_default_group_config(
    config: &mut PermissionGroupsConfig,
    group: &str,
) -> Result<bool, PermissionGroupEditError> {
    if !config.groups.contains_key(group) {
        return Err(PermissionGroupEditError::Missing(group.to_owned()));
    }
    if config.default_groups.iter().any(|default| default == group) {
        return Ok(false);
    }

    config.default_groups.push(group.to_owned());
    Ok(true)
}

pub(super) fn remove_default_group_config(
    config: &mut PermissionGroupsConfig,
    group: &str,
) -> Result<bool, PermissionGroupEditError> {
    if !config.groups.contains_key(group) {
        return Err(PermissionGroupEditError::Missing(group.to_owned()));
    }
    let old_len = config.default_groups.len();
    config.default_groups.retain(|default| default != group);
    Ok(config.default_groups.len() != old_len)
}

pub(super) fn set_group_config_permission(
    config: &mut PermissionGroupsConfig,
    group: &str,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
    state: PermissionState,
) -> Result<bool, PermissionGroupEditError> {
    let Some(group_config) = config.groups.get_mut(group) else {
        return Err(PermissionGroupEditError::Missing(group.to_owned()));
    };
    let states = group_config_permission_states(group_config, permission, rule_context);
    if states.len() == 1 && states[0] == state {
        return Ok(false);
    }

    remove_group_config_permission(group_config, permission, rule_context);
    push_group_config_permission(group_config, permission, rule_context, state)?;
    Ok(true)
}

pub(super) fn unset_group_config_permission(
    config: &mut PermissionGroupsConfig,
    group: &str,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
) -> Result<bool, PermissionGroupEditError> {
    let Some(group_config) = config.groups.get_mut(group) else {
        return Err(PermissionGroupEditError::Missing(group.to_owned()));
    };

    Ok(remove_group_config_permission(
        group_config,
        permission,
        rule_context,
    ))
}

pub(super) fn set_group_config_metadata(
    config: &mut PermissionGroupsConfig,
    group: &str,
    key: &Identifier,
    value: &PermissionMetadataValue,
    rule_context: &PermissionRuleContext,
) -> Result<bool, PermissionGroupEditError> {
    let Some(group_config) = config.groups.get_mut(group) else {
        return Err(PermissionGroupEditError::Missing(group.to_owned()));
    };
    if group_config_metadata_value(group_config, key, rule_context) == Some(value) {
        return Ok(false);
    }

    remove_group_config_metadata(group_config, key, rule_context);
    push_group_config_metadata(group_config, key, value, rule_context)?;
    Ok(true)
}

pub(super) fn unset_group_config_metadata(
    config: &mut PermissionGroupsConfig,
    group: &str,
    key: &Identifier,
    rule_context: &PermissionRuleContext,
) -> Result<bool, PermissionGroupEditError> {
    let Some(group_config) = config.groups.get_mut(group) else {
        return Err(PermissionGroupEditError::Missing(group.to_owned()));
    };

    Ok(remove_group_config_metadata(
        group_config,
        key,
        rule_context,
    ))
}

pub(super) fn set_group_config_priority(
    config: &mut PermissionGroupsConfig,
    group: &str,
    priority: i32,
) -> Result<bool, PermissionGroupEditError> {
    let Some(group_config) = config.groups.get_mut(group) else {
        return Err(PermissionGroupEditError::Missing(group.to_owned()));
    };
    if group_config.priority == priority {
        return Ok(false);
    }

    group_config.priority = priority;
    Ok(true)
}

fn push_group_config_permission(
    group_config: &mut PermissionGroupConfig,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
    state: PermissionState,
) -> Result<(), PermissionGroupEditError> {
    let expression = PermissionRuleExpression::new(permission.clone(), rule_context.clone());
    match state {
        PermissionState::Allow => group_config.allow.push(expression.to_string()),
        PermissionState::Deny => group_config.deny.push(expression.to_string()),
    }
    Ok(())
}

fn push_group_config_metadata(
    group_config: &mut PermissionGroupConfig,
    key: &Identifier,
    value: &PermissionMetadataValue,
    rule_context: &PermissionRuleContext,
) -> Result<(), PermissionGroupEditError> {
    group_config.metadata.push(PermissionMetadataRuleConfig {
        key: PermissionMetadataExpression::new(key.clone(), rule_context.clone()).to_string(),
        value: value.clone(),
    });
    Ok(())
}

pub(super) fn group_config_permission_states(
    group_config: &PermissionGroupConfig,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
) -> Vec<PermissionState> {
    let mut states = Vec::new();
    states.extend(
        group_config
            .allow
            .iter()
            .filter(|expression| permission_rule_expression_matches(expression, permission, rule_context))
            .map(|_| PermissionState::Allow),
    );
    states.extend(
        group_config
            .deny
            .iter()
            .filter(|expression| permission_rule_expression_matches(expression, permission, rule_context))
            .map(|_| PermissionState::Deny),
    );

    states
}

pub(super) fn group_config_metadata_value<'a>(
    group_config: &'a PermissionGroupConfig,
    key: &Identifier,
    rule_context: &PermissionRuleContext,
) -> Option<&'a PermissionMetadataValue> {
    group_config
        .metadata
        .iter()
        .find(|value| {
            permission_metadata_expression_matches(&value.key, key, rule_context)
        })
        .map(|value| &value.value)
}

fn remove_group_config_permission(
    group_config: &mut PermissionGroupConfig,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
) -> bool {
    let old_allow_len = group_config.allow.len();
    group_config.allow.retain(|expression| {
        !permission_rule_expression_matches(expression, permission, rule_context)
    });
    let allow_changed = group_config.allow.len() != old_allow_len;

    let old_deny_len = group_config.deny.len();
    group_config.deny.retain(|expression| {
        !permission_rule_expression_matches(expression, permission, rule_context)
    });

    allow_changed || group_config.deny.len() != old_deny_len
}

fn remove_group_config_metadata(
    group_config: &mut PermissionGroupConfig,
    key: &Identifier,
    rule_context: &PermissionRuleContext,
) -> bool {
    let old_len = group_config.metadata.len();
    group_config.metadata.retain(|value| {
        !permission_metadata_expression_matches(&value.key, key, rule_context)
    });
    group_config.metadata.len() != old_len
}

fn permission_rule_expression_matches(
    expression: &str,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
) -> bool {
    PermissionRuleExpression::parse(expression.to_owned()).is_ok_and(|expression| {
        expression.key() == permission && expression.context() == rule_context
    })
}

fn permission_metadata_expression_matches(
    expression: &str,
    key: &Identifier,
    rule_context: &PermissionRuleContext,
) -> bool {
    PermissionMetadataExpression::parse(expression.to_owned()).is_ok_and(|expression| {
        expression.key() == key && expression.context() == rule_context
    })
}
