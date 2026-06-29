//! Group permission config editing helpers for the `steelperms` command.

use std::fmt;

use steel_utils::Identifier;

use crate::permission::{
    OP_GROUP, PermissionGroupConfig, PermissionGroupsConfig, PermissionKey, PermissionRuleConfig,
    PermissionRuleContext, PermissionRuleContextConfig, PermissionRuleCustomContextConfig,
    PermissionRuleStateConfig, PermissionState, PermissionValue, PermissionValueRuleConfig,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum PermissionGroupEditError {
    AlreadyExists(String),
    Missing(String),
    Required(String),
    Default(String),
    UnsupportedContext(PermissionRuleContext),
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
            Self::UnsupportedContext(context) => {
                write!(
                    f,
                    "permission group config cannot store context '{context}'"
                )
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
    value: &PermissionValue,
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
    if rule_context.is_global() {
        match state {
            PermissionState::Allow => group_config.allow.push(permission.as_str().to_owned()),
            PermissionState::Deny => group_config.deny.push(permission.as_str().to_owned()),
        }
        return Ok(());
    }

    group_config.rules.push(PermissionRuleConfig {
        key: permission.as_str().to_owned(),
        state: permission_rule_state_config(state),
        context: Some(permission_rule_context_config(rule_context)?),
    });
    Ok(())
}

fn push_group_config_metadata(
    group_config: &mut PermissionGroupConfig,
    key: &Identifier,
    value: &PermissionValue,
    rule_context: &PermissionRuleContext,
) -> Result<(), PermissionGroupEditError> {
    group_config.values.push(PermissionValueRuleConfig {
        key: key.to_string(),
        value: value.clone(),
        context: if rule_context.is_global() {
            None
        } else {
            Some(permission_rule_context_config(rule_context)?)
        },
    });
    Ok(())
}

pub(super) fn group_config_permission_states(
    group_config: &PermissionGroupConfig,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
) -> Vec<PermissionState> {
    let mut states = Vec::new();
    if rule_context.is_global() {
        states.extend(
            group_config
                .allow
                .iter()
                .filter(|key| key.as_str() == permission.as_str())
                .map(|_| PermissionState::Allow),
        );
        states.extend(
            group_config
                .deny
                .iter()
                .filter(|key| key.as_str() == permission.as_str())
                .map(|_| PermissionState::Deny),
        );
    }
    states.extend(
        group_config
            .rules
            .iter()
            .filter(|rule| {
                rule.key == permission.as_str()
                    && permission_rule_config_matches(rule.context.as_ref(), rule_context)
            })
            .map(|rule| permission_state_from_rule_config(rule.state)),
    );

    states
}

pub(super) fn group_config_metadata_value<'a>(
    group_config: &'a PermissionGroupConfig,
    key: &Identifier,
    rule_context: &PermissionRuleContext,
) -> Option<&'a PermissionValue> {
    group_config
        .values
        .iter()
        .find(|value| {
            value.key == key.to_string()
                && permission_rule_config_matches(value.context.as_ref(), rule_context)
        })
        .map(|value| &value.value)
}

fn remove_group_config_permission(
    group_config: &mut PermissionGroupConfig,
    permission: &PermissionKey,
    rule_context: &PermissionRuleContext,
) -> bool {
    let mut changed = false;
    if rule_context.is_global() {
        let old_allow_len = group_config.allow.len();
        group_config
            .allow
            .retain(|key| key.as_str() != permission.as_str());
        changed |= group_config.allow.len() != old_allow_len;

        let old_deny_len = group_config.deny.len();
        group_config
            .deny
            .retain(|key| key.as_str() != permission.as_str());
        changed |= group_config.deny.len() != old_deny_len;
    }

    let old_rules_len = group_config.rules.len();
    group_config.rules.retain(|rule| {
        rule.key != permission.as_str()
            || !permission_rule_config_matches(rule.context.as_ref(), rule_context)
    });
    changed | (group_config.rules.len() != old_rules_len)
}

fn remove_group_config_metadata(
    group_config: &mut PermissionGroupConfig,
    key: &Identifier,
    rule_context: &PermissionRuleContext,
) -> bool {
    let old_len = group_config.values.len();
    group_config.values.retain(|value| {
        value.key != key.to_string()
            || !permission_rule_config_matches(value.context.as_ref(), rule_context)
    });
    group_config.values.len() != old_len
}

fn permission_rule_context_config(
    rule_context: &PermissionRuleContext,
) -> Result<PermissionRuleContextConfig, PermissionGroupEditError> {
    let mut config = PermissionRuleContextConfig::default();
    append_permission_rule_context_config(&mut config, rule_context)?;
    if config.domain.is_none() && config.world.is_none() && config.custom.is_empty() {
        return Err(PermissionGroupEditError::UnsupportedContext(
            rule_context.clone(),
        ));
    }
    Ok(config)
}

fn append_permission_rule_context_config(
    config: &mut PermissionRuleContextConfig,
    rule_context: &PermissionRuleContext,
) -> Result<(), PermissionGroupEditError> {
    match rule_context {
        PermissionRuleContext::Global => {}
        PermissionRuleContext::Domain(domain) => config.domain = Some(domain.clone()),
        PermissionRuleContext::World(world) => config.world = Some(world.to_string()),
        PermissionRuleContext::Custom { key, value } => {
            config.custom.push(PermissionRuleCustomContextConfig {
                key: key.as_str().to_owned(),
                value: value.clone(),
            });
        }
        PermissionRuleContext::All(contexts) => {
            for context in contexts.iter() {
                append_permission_rule_context_config(config, context)?;
            }
        }
    }
    Ok(())
}

fn permission_rule_config_matches(
    config: Option<&PermissionRuleContextConfig>,
    rule_context: &PermissionRuleContext,
) -> bool {
    match config {
        None => rule_context.is_global(),
        Some(config) => config
            .clone()
            .into_rule_context()
            .is_ok_and(|context| &context == rule_context),
    }
}

fn permission_rule_state_config(state: PermissionState) -> PermissionRuleStateConfig {
    match state {
        PermissionState::Allow => PermissionRuleStateConfig::Allow,
        PermissionState::Deny => PermissionRuleStateConfig::Deny,
    }
}

pub(super) fn permission_state_from_rule_config(state: PermissionRuleStateConfig) -> PermissionState {
    match state {
        PermissionRuleStateConfig::Allow => PermissionState::Allow,
        PermissionRuleStateConfig::Deny => PermissionState::Deny,
    }
}
