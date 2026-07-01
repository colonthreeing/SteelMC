//! Permission-management checks and suggestion filtering for `steelperms`.

use std::collections::BTreeSet;

use steel_protocol::packets::game::SuggestionEntry;
use steel_utils::Identifier;

use crate::command::error::CommandError;
use crate::command::requirement::{PermissionExpr, RequirementContext};
use crate::permission::{
    PermissionEntry, PermissionGroupConfig, PermissionKey, PermissionKeyError,
    PermissionMetadataCatalog, PermissionMetadataExpression, PermissionRuleConfig,
    PermissionRuleContext, PermissionRuleExpression, PermissionSegment, PermissionSet,
    PermissionValueEntry, PermissionValueRuleConfig, PermissionValueSet, parse_permission_value_key,
};

pub(super) fn direct_permission_override_suggestions(
    prefix: &str,
    overrides: impl IntoIterator<Item = PermissionSet>,
    context: &dyn RequirementContext,
) -> Vec<SuggestionEntry> {
    let mut permissions = BTreeSet::new();
    for overrides in overrides {
        for entry in overrides.entries() {
            let expression =
                PermissionRuleExpression::new(entry.key().clone(), entry.context().clone())
                    .to_string();
            if expression.starts_with(prefix) && can_manage_permission(context, entry.key()) {
                permissions.insert(expression);
            }
        }
    }

    permissions.into_iter().map(SuggestionEntry::new).collect()
}

pub(super) fn direct_metadata_override_suggestions(
    prefix: &str,
    values: impl IntoIterator<Item = PermissionValueSet>,
    context: &dyn RequirementContext,
) -> Vec<SuggestionEntry> {
    let mut keys = BTreeSet::new();
    for values in values {
        for entry in values.entries() {
            let expression =
                PermissionMetadataExpression::new(entry.key().clone(), entry.context().clone())
                    .to_string();
            if expression.starts_with(prefix) && can_manage_metadata(context, entry.key()) {
                keys.insert(expression);
            }
        }
    }

    keys.into_iter().map(SuggestionEntry::new).collect()
}

pub(super) fn group_permission_suggestions(
    prefix: &str,
    group_config: &PermissionGroupConfig,
    context: &dyn RequirementContext,
) -> Vec<SuggestionEntry> {
    let mut permissions = BTreeSet::new();
    for permission in &group_config.allow {
        push_managed_group_permission_suggestion(&mut permissions, prefix, permission, context);
    }
    for permission in &group_config.deny {
        push_managed_group_permission_suggestion(&mut permissions, prefix, permission, context);
    }
    for rule in &group_config.rules {
        push_managed_group_permission_rule_suggestion(&mut permissions, prefix, rule, context);
    }

    permissions.into_iter().map(SuggestionEntry::new).collect()
}

pub(super) fn manageable_assigned_groups(
    groups: &[String],
    context: &dyn RequirementContext,
) -> Vec<String> {
    groups
        .iter()
        .filter(|group| can_manage_group(context, group))
        .cloned()
        .collect()
}

pub(super) fn manageable_permission_entries(
    entries: &[PermissionEntry],
    context: &dyn RequirementContext,
) -> Vec<PermissionEntry> {
    entries
        .iter()
        .filter(|entry| can_manage_permission(context, entry.key()))
        .cloned()
        .collect()
}

pub(super) fn manageable_metadata_entries(
    entries: &[PermissionValueEntry],
    context: &dyn RequirementContext,
) -> Vec<PermissionValueEntry> {
    entries
        .iter()
        .filter(|entry| can_manage_metadata(context, entry.key()))
        .cloned()
        .collect()
}

pub(super) fn manageable_group_permission_keys(
    permissions: &[String],
    context: &dyn RequirementContext,
) -> Vec<String> {
    permissions
        .iter()
        .filter(|permission| {
            PermissionKey::parse((*permission).clone())
                .is_ok_and(|permission| can_manage_permission(context, &permission))
        })
        .cloned()
        .collect()
}

pub(super) fn manageable_group_permission_rules(
    rules: &[PermissionRuleConfig],
    context: &dyn RequirementContext,
) -> Vec<PermissionRuleConfig> {
    rules
        .iter()
        .filter(|rule| {
            PermissionKey::parse(rule.key.clone())
                .is_ok_and(|permission| can_manage_permission(context, &permission))
        })
        .cloned()
        .collect()
}

pub(super) fn manageable_group_metadata_rules(
    values: &[PermissionValueRuleConfig],
    context: &dyn RequirementContext,
) -> Vec<PermissionValueRuleConfig> {
    values
        .iter()
        .filter(|value| {
            parse_permission_value_key(value.key.clone())
                .is_ok_and(|key| can_manage_metadata(context, &key))
        })
        .cloned()
        .collect()
}

pub(super) fn group_metadata_suggestions(
    prefix: &str,
    group_config: &PermissionGroupConfig,
    context: &dyn RequirementContext,
) -> Vec<SuggestionEntry> {
    let mut keys = BTreeSet::new();
    for value in &group_config.values {
        let Ok(key) = parse_permission_value_key(value.key.clone()) else {
            continue;
        };
        let rule_context = value
            .context
            .clone()
            .map_or(Ok(PermissionRuleContext::Global), |context| {
                context.into_rule_context()
            });
        let Ok(rule_context) = rule_context else {
            continue;
        };
        let expression = PermissionMetadataExpression::new(key.clone(), rule_context).to_string();
        if expression.starts_with(prefix) && can_manage_metadata(context, &key) {
            keys.insert(expression);
        }
    }

    keys.into_iter().map(SuggestionEntry::new).collect()
}

pub(super) fn metadata_catalog_suggestions(
    prefix: &str,
    catalog: &PermissionMetadataCatalog,
    context: &dyn RequirementContext,
) -> Vec<SuggestionEntry> {
    catalog
        .suggestions(prefix)
        .into_iter()
        .filter_map(|key| {
            let Ok(parsed_key) = parse_permission_value_key(key.clone()) else {
                return None;
            };
            can_manage_metadata(context, &parsed_key).then(|| SuggestionEntry::new(key))
        })
        .collect()
}

fn push_managed_group_permission_suggestion(
    permissions: &mut BTreeSet<String>,
    prefix: &str,
    permission: &str,
    context: &dyn RequirementContext,
) {
    if !permission.starts_with(prefix) {
        return;
    }
    let Ok(permission) = PermissionKey::parse(permission) else {
        return;
    };
    if can_manage_permission(context, &permission) {
        permissions.insert(permission.as_str().to_owned());
    }
}

fn push_managed_group_permission_rule_suggestion(
    permissions: &mut BTreeSet<String>,
    prefix: &str,
    rule: &PermissionRuleConfig,
    context: &dyn RequirementContext,
) {
    let Ok(permission) = PermissionKey::parse(rule.key.clone()) else {
        return;
    };
    if !can_manage_permission(context, &permission) {
        return;
    }

    let rule_context = rule
        .context
        .clone()
        .map_or(Ok(PermissionRuleContext::Global), |context| {
            context.into_rule_context()
        });
    let Ok(rule_context) = rule_context else {
        return;
    };
    let expression = PermissionRuleExpression::new(permission, rule_context).to_string();
    if expression.starts_with(prefix) {
        permissions.insert(expression);
    }
}

pub(super) fn require_permission_management(
    context: &dyn RequirementContext,
    permission: &PermissionKey,
) -> Result<(), CommandError> {
    if can_manage_permission(context, permission) {
        Ok(())
    } else {
        Err(CommandError::PermissionDenied)
    }
}

pub(super) fn can_manage_permission(context: &dyn RequirementContext, permission: &PermissionKey) -> bool {
    let Ok(management_permission) = permission_management_key(permission) else {
        return false;
    };
    context.has_permission(&PermissionExpr::key(management_permission))
}

pub(super) fn require_metadata_management(
    context: &dyn RequirementContext,
    key: &Identifier,
) -> Result<(), CommandError> {
    if can_manage_metadata(context, key) {
        Ok(())
    } else {
        Err(CommandError::PermissionDenied)
    }
}

pub(super) fn can_manage_metadata(context: &dyn RequirementContext, key: &Identifier) -> bool {
    let Ok(management_permission) = metadata_management_key(key) else {
        return false;
    };
    context.has_permission(&PermissionExpr::key(management_permission))
}

pub(super) fn metadata_management_key(key: &Identifier) -> Result<PermissionKey, PermissionKeyError> {
    let mut segments = vec![
        PermissionSegment::parse("steel")?,
        PermissionSegment::parse("permission")?,
        PermissionSegment::parse("metadata")?,
    ];
    push_metadata_permission_segments(&mut segments, key.namespace.as_ref())?;
    push_metadata_permission_segments(&mut segments, key.path.as_ref())?;
    PermissionKey::from_segments(segments)
}

fn push_metadata_permission_segments(
    segments: &mut Vec<PermissionSegment>,
    value: &str,
) -> Result<(), PermissionKeyError> {
    for segment in value.split(['.', '/']) {
        segments.push(PermissionSegment::parse(segment)?);
    }
    Ok(())
}

fn permission_management_key(
    permission: &PermissionKey,
) -> Result<PermissionKey, PermissionKeyError> {
    PermissionKey::parse(format!("steel.permission.manage.{}", permission.as_str()))
}

pub(super) fn require_group_management(
    context: &dyn RequirementContext,
    group: &str,
) -> Result<(), CommandError> {
    if can_manage_group(context, group) {
        Ok(())
    } else {
        Err(CommandError::PermissionDenied)
    }
}

pub(super) fn can_manage_group(context: &dyn RequirementContext, group: &str) -> bool {
    let Ok(group) = PermissionSegment::parse(group) else {
        return false;
    };
    let Ok(permission) = PermissionKey::parse(format!("steel.permission.group.{}", group.as_str()))
    else {
        return false;
    };
    context.has_permission(&PermissionExpr::key(permission))
}

pub(super) fn assigned_group_suggestions(
    prefix: &str,
    groups: impl IntoIterator<Item = Vec<String>>,
    context: &dyn RequirementContext,
) -> Vec<SuggestionEntry> {
    let mut assigned_groups = BTreeSet::new();
    for groups in groups {
        for group in groups {
            if group.starts_with(prefix) && can_manage_group(context, &group) {
                assigned_groups.insert(group);
            }
        }
    }

    assigned_groups
        .into_iter()
        .map(SuggestionEntry::new)
        .collect()
}
