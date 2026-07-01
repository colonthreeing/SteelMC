use std::sync::Arc;

use steel_protocol::packets::game::SuggestionEntry;

use super::node::{CommandNode, CommandNodeExecutor, CommandNodeKind, CommandRedirectModifier};
use super::{
    CommandArgumentParser, CommandParseError, CommandParseErrorKind, CommandRedirectTarget,
    DynamicPermission, ParseResults, ParsedArgument, ParsedArguments, ParsedCommandAction,
    ParsedRedirect, SuggestionResult, dynamic_permissions_allow,
};
use crate::command::{
    reader::{ARGUMENT_SEPARATOR, CommandReader},
    requirement::CommandInputContext,
};

pub(super) fn parse_children(
    input: &str,
    reader: &mut CommandReader<'_>,
    children: &[CommandNode],
    arguments: ParsedArguments,
    path: Vec<String>,
    dynamic_permissions: Vec<DynamicPermission>,
    context: &dyn CommandInputContext,
    roots: &[CommandNode],
    current_root_children: Option<&[CommandNode]>,
) -> Result<ParseResults, CommandParseError> {
    let mut best_error = None;
    let mut usable_child_seen = false;

    for child_index in relevant_child_indexes(children, reader) {
        let child = &children[child_index];
        if !child.requirement.allows(context) {
            continue;
        }

        let mut child_reader = reader.clone();
        let mut child_arguments = arguments.clone();
        let mut child_path = path.clone();
        let mut child_dynamic_permissions = dynamic_permissions.clone();

        match child.parse_self(&mut child_reader, &mut child_arguments, context) {
            Ok(()) => {
                child_path.push(child.display_name().to_owned());
                child_dynamic_permissions.extend(child.dynamic_permissions.iter().cloned());
                match dynamic_permissions_allow(
                    &child_dynamic_permissions,
                    &child_arguments,
                    context,
                ) {
                    Ok(true) => {}
                    Ok(false) => continue,
                    Err(error) => {
                        keep_best_error(
                            &mut best_error,
                            dynamic_permission_parse_error(error, child_reader.absolute_cursor()),
                        );
                        continue;
                    }
                }
                usable_child_seen = true;
                let child_root_children = current_root_children.or_else(|| {
                    matches!(&child.kind, CommandNodeKind::Literal(_))
                        .then_some(child.children.as_slice())
                });
                match parse_after_node(
                    input,
                    child,
                    &mut child_reader,
                    child_arguments,
                    child_path,
                    child_dynamic_permissions,
                    context,
                    roots,
                    child_root_children,
                ) {
                    Ok(result) => return Ok(result),
                    Err(error) => keep_best_error(&mut best_error, error),
                }
            }
            Err(error) => keep_best_error(&mut best_error, error),
        }
    }

    Err(best_error.unwrap_or_else(|| {
        CommandParseError::new(
            if usable_child_seen {
                CommandParseErrorKind::ExpectedArgument
            } else {
                CommandParseErrorKind::UnknownCommand
            },
            reader.absolute_cursor(),
        )
    }))
}

fn parse_after_node(
    input: &str,
    node: &CommandNode,
    reader: &mut CommandReader<'_>,
    arguments: ParsedArguments,
    path: Vec<String>,
    dynamic_permissions: Vec<DynamicPermission>,
    context: &dyn CommandInputContext,
    roots: &[CommandNode],
    current_root_children: Option<&[CommandNode]>,
) -> Result<ParseResults, CommandParseError> {
    if !reader.can_read() {
        return node
            .executable(input, arguments, path, dynamic_permissions)
            .ok_or_else(|| {
                CommandParseError::new(
                    CommandParseErrorKind::IncompleteCommand,
                    reader.absolute_cursor(),
                )
            });
    }

    if !reader.is_argument_separator() {
        return Err(CommandParseError::new(
            CommandParseErrorKind::TrailingData,
            reader.absolute_cursor(),
        ));
    }

    reader.expect_argument_separator()?;
    if !reader.can_read() {
        return node
            .executable(input, arguments, path, dynamic_permissions)
            .ok_or_else(|| {
                CommandParseError::new(
                    CommandParseErrorKind::IncompleteCommand,
                    reader.absolute_cursor(),
                )
            });
    }

    if node.redirect.is_some() {
        validate_redirect_tail(
            input,
            reader,
            node,
            &path,
            context,
            roots,
            current_root_children,
        )?;
        return node.redirectable(
            input,
            arguments,
            path,
            dynamic_permissions,
            reader.remaining().to_owned(),
        );
    }

    if node.children.is_empty() {
        return Err(CommandParseError::new(
            CommandParseErrorKind::TrailingData,
            reader.absolute_cursor(),
        ));
    }

    parse_children(
        input,
        reader,
        &node.children,
        arguments,
        path,
        dynamic_permissions,
        context,
        roots,
        current_root_children,
    )
}

fn validate_redirect_tail(
    input: &str,
    reader: &CommandReader<'_>,
    node: &CommandNode,
    path: &[String],
    context: &dyn CommandInputContext,
    roots: &[CommandNode],
    current_root_children: Option<&[CommandNode]>,
) -> Result<(), CommandParseError> {
    let Some(redirect) = &node.redirect else {
        return Ok(());
    };

    // Brigadier validates redirected command tails during parse. Keep the
    // original redirect action, but reject tails that the target graph cannot use.
    let mut redirect_reader =
        CommandReader::with_offset(reader.remaining(), reader.absolute_cursor());
    match redirect.target {
        CommandRedirectTarget::All => {
            parse_children(
                input,
                &mut redirect_reader,
                roots,
                ParsedArguments::default(),
                Vec::new(),
                Vec::new(),
                context,
                roots,
                None,
            )?;
        }
        CommandRedirectTarget::Current => {
            let Some(current_root) = path.first() else {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::IncompleteCommand,
                    reader.absolute_cursor(),
                ));
            };
            let Some(current_root_children) = current_root_children else {
                return Err(CommandParseError::new(
                    CommandParseErrorKind::IncompleteCommand,
                    reader.absolute_cursor(),
                ));
            };
            parse_children(
                input,
                &mut redirect_reader,
                current_root_children,
                ParsedArguments::default(),
                vec![current_root.clone()],
                Vec::new(),
                context,
                roots,
                Some(current_root_children),
            )?;
        }
    }

    Ok(())
}

fn keep_best_error(best_error: &mut Option<CommandParseError>, error: CommandParseError) {
    if best_error
        .as_ref()
        .is_none_or(|best| error.is_better_than(best))
    {
        *best_error = Some(error);
    }
}

fn dynamic_permission_parse_error(
    error: impl std::fmt::Display,
    cursor: usize,
) -> CommandParseError {
    CommandParseError::new(
        CommandParseErrorKind::DynamicPermissionResolution(error.to_string()),
        cursor,
    )
}

fn dynamic_permissions_allowed(
    permissions: &[DynamicPermission],
    arguments: &ParsedArguments,
    context: &dyn CommandInputContext,
) -> bool {
    matches!(
        dynamic_permissions_allow(permissions, arguments, context),
        Ok(true)
    )
}

pub(super) fn suggest_children(
    reader: &CommandReader<'_>,
    children: &[CommandNode],
    arguments: ParsedArguments,
    dynamic_permissions: Vec<DynamicPermission>,
    context: &dyn CommandInputContext,
    roots: &[CommandNode],
    current_root_children: Option<&[CommandNode]>,
) -> Option<SuggestionResult> {
    if !dynamic_permissions_allowed(&dynamic_permissions, &arguments, context) {
        return None;
    }

    let relevant_child_indexes = relevant_child_indexes(children, reader);
    let mut best_result = None;

    for (child_index, child) in children.iter().enumerate() {
        if !child.requirement.allows(context) {
            continue;
        }

        let child_root_children = current_root_children.or_else(|| {
            matches!(&child.kind, CommandNodeKind::Literal(_)).then_some(child.children.as_slice())
        });

        if let Some(result) = child.suggest(
            reader,
            arguments.clone(),
            dynamic_permissions.clone(),
            context,
            roots,
            child_root_children,
            relevant_child_indexes.contains(&child_index),
        ) {
            keep_best_suggestion(&mut best_result, result);
        }
    }

    best_result
}

fn keep_best_suggestion(best_result: &mut Option<SuggestionResult>, result: SuggestionResult) {
    let Some(best) = best_result else {
        *best_result = Some(result);
        return;
    };

    if result.start > best.start {
        *best = result;
        return;
    }

    if result.start == best.start && result.length == best.length {
        best.suggestions.extend(result.suggestions);
    }
}

fn relevant_child_indexes(children: &[CommandNode], reader: &CommandReader<'_>) -> Vec<usize> {
    let has_literal_children = children
        .iter()
        .any(|child| matches!(&child.kind, CommandNodeKind::Literal(_)));
    if !has_literal_children {
        return (0..children.len()).collect();
    }

    let token = next_token(reader);
    if let Some(index) = children
        .iter()
        .position(|child| matches!(&child.kind, CommandNodeKind::Literal(name) if name == token))
    {
        return vec![index];
    }

    children
        .iter()
        .enumerate()
        .filter_map(|(index, child)| {
            matches!(&child.kind, CommandNodeKind::Argument { .. }).then_some(index)
        })
        .collect()
}

fn next_token<'a>(reader: &CommandReader<'a>) -> &'a str {
    let remaining = reader.remaining();
    let end = remaining
        .char_indices()
        .find_map(|(index, ch)| (ch == ARGUMENT_SEPARATOR).then_some(index))
        .unwrap_or(remaining.len());
    &remaining[..end]
}

fn make_suggestion_result(
    reader: &CommandReader<'_>,
    suggestions: Vec<SuggestionEntry>,
) -> Option<SuggestionResult> {
    if suggestions.is_empty() {
        return None;
    }

    let token = suggestion_token(reader);
    Some(SuggestionResult {
        suggestions,
        start: token.start,
        length: token.length,
    })
}

fn filter_argument_suggestions_by_dynamic_permissions(
    suggestions: Vec<SuggestionEntry>,
    parser: &dyn CommandArgumentParser,
    argument_name: &str,
    arguments: &ParsedArguments,
    dynamic_permissions: &[DynamicPermission],
    context: &dyn CommandInputContext,
) -> Vec<SuggestionEntry> {
    if dynamic_permissions.is_empty() {
        return suggestions;
    }

    suggestions
        .into_iter()
        .filter(|suggestion| {
            let mut reader = CommandReader::new(&suggestion.text);
            let Ok(value) = parse_argument(parser, argument_name, &mut reader, context) else {
                return false;
            };
            reader.skip_whitespace();
            if reader.can_read() {
                return false;
            }

            let mut arguments = arguments.clone();
            arguments.insert(argument_name, value);
            dynamic_permissions_allowed(dynamic_permissions, &arguments, context)
        })
        .collect()
}

fn suggestion_token(reader: &CommandReader<'_>) -> SuggestionToken {
    let remaining = reader.remaining();
    let prefix = remaining
        .chars()
        .take_while(|ch| *ch != ARGUMENT_SEPARATOR)
        .collect::<String>();
    let length = prefix.encode_utf16().count() as i32;
    let is_at_end = remaining[prefix.len()..].is_empty();

    SuggestionToken {
        prefix,
        start: reader.absolute_utf16_cursor() as i32,
        length,
        is_at_end,
    }
}

struct SuggestionToken {
    prefix: String,
    start: i32,
    length: i32,
    is_at_end: bool,
}

impl CommandNode {
    fn parse_self(
        &self,
        reader: &mut CommandReader<'_>,
        arguments: &mut ParsedArguments,
        context: &dyn CommandInputContext,
    ) -> Result<(), CommandParseError> {
        match &self.kind {
            CommandNodeKind::Literal(expected) => {
                let cursor = reader.absolute_cursor();
                if reader.read_literal(expected) {
                    Ok(())
                } else {
                    Err(CommandParseError::new(
                        CommandParseErrorKind::ExpectedLiteral(expected.clone()),
                        cursor,
                    ))
                }
            }
            CommandNodeKind::Argument { name, parser } => {
                let value = parse_argument(parser.as_ref(), name, reader, context)?;
                arguments.insert(name, value);
                Ok(())
            }
        }
    }

    fn executable(
        &self,
        input: &str,
        arguments: ParsedArguments,
        path: Vec<String>,
        dynamic_permissions: Vec<DynamicPermission>,
    ) -> Option<ParseResults> {
        let action = match self.executor.as_ref()? {
            CommandNodeExecutor::Result(executor) => {
                ParsedCommandAction::Execute(Arc::clone(executor))
            }
            CommandNodeExecutor::Step(executor) => {
                ParsedCommandAction::ExecuteStep(Arc::clone(executor))
            }
        };
        Some(ParseResults {
            input: input.to_owned(),
            arguments,
            path,
            dynamic_permissions,
            action,
        })
    }

    fn redirectable(
        &self,
        input: &str,
        arguments: ParsedArguments,
        path: Vec<String>,
        dynamic_permissions: Vec<DynamicPermission>,
        command: String,
    ) -> Result<ParseResults, CommandParseError> {
        let Some(redirect) = &self.redirect else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::TrailingData,
                input.len(),
            ));
        };
        let Some(current_root) = path.first().cloned() else {
            return Err(CommandParseError::new(
                CommandParseErrorKind::IncompleteCommand,
                input.len(),
            ));
        };

        Ok(ParseResults {
            input: input.to_owned(),
            arguments,
            path,
            dynamic_permissions,
            action: ParsedCommandAction::Redirect(ParsedRedirect {
                target: redirect.target,
                current_root,
                command,
                modifier: match &redirect.modifier {
                    CommandRedirectModifier::Single(executor) => {
                        super::ParsedRedirectModifier::Single(Arc::clone(executor))
                    }
                    CommandRedirectModifier::Fork(executor) => {
                        super::ParsedRedirectModifier::Fork(Arc::clone(executor))
                    }
                },
                returns: redirect.returns,
            }),
        })
    }

    fn suggest(
        &self,
        reader: &CommandReader<'_>,
        mut arguments: ParsedArguments,
        mut dynamic_permissions: Vec<DynamicPermission>,
        context: &dyn CommandInputContext,
        roots: &[CommandNode],
        current_root_children: Option<&[CommandNode]>,
        allow_deeper_suggestions: bool,
    ) -> Option<SuggestionResult> {
        let token = suggestion_token(reader);
        let direct_suggestions = match &self.kind {
            CommandNodeKind::Literal(name)
                if token.is_at_end && name.starts_with(&token.prefix) =>
            {
                dynamic_permissions_allow(&self.dynamic_permissions, &arguments, context)
                    .ok()
                    .filter(|allowed| *allowed)
                    .and_then(|_| {
                        make_suggestion_result(reader, vec![SuggestionEntry::new(name.clone())])
                    })
            }
            CommandNodeKind::Argument { name, parser } if token.is_at_end => {
                let suggestions = parser.suggest(&token.prefix, &arguments, context);
                let suggestions = filter_argument_suggestions_by_dynamic_permissions(
                    suggestions,
                    parser.as_ref(),
                    name,
                    &arguments,
                    &self.dynamic_permissions,
                    context,
                );
                make_suggestion_result(reader, suggestions)
            }
            CommandNodeKind::Literal(_) | CommandNodeKind::Argument { .. } => None,
        };

        if !allow_deeper_suggestions {
            return direct_suggestions;
        }

        let mut parsed_reader = reader.clone();
        if self
            .parse_self(&mut parsed_reader, &mut arguments, context)
            .is_err()
        {
            return direct_suggestions;
        }

        dynamic_permissions.extend(self.dynamic_permissions.iter().cloned());
        if !dynamic_permissions_allowed(&dynamic_permissions, &arguments, context) {
            return direct_suggestions;
        }

        let mut result = direct_suggestions;
        if let Some(deeper) = self.suggest_after_successful_parse(
            &mut parsed_reader,
            arguments,
            dynamic_permissions,
            context,
            roots,
            current_root_children,
        ) {
            keep_best_suggestion(&mut result, deeper);
        }
        result
    }

    fn suggest_after_successful_parse(
        &self,
        reader: &mut CommandReader<'_>,
        arguments: ParsedArguments,
        dynamic_permissions: Vec<DynamicPermission>,
        context: &dyn CommandInputContext,
        roots: &[CommandNode],
        current_root_children: Option<&[CommandNode]>,
    ) -> Option<SuggestionResult> {
        if !reader.can_read() {
            return None;
        }

        if !reader.is_argument_separator() {
            return None;
        }

        reader.expect_argument_separator().ok()?;
        if let Some(redirect) = &self.redirect {
            return match redirect.target {
                CommandRedirectTarget::Current => current_root_children.and_then(|children| {
                    suggest_children(
                        reader,
                        children,
                        ParsedArguments::default(),
                        Vec::new(),
                        context,
                        roots,
                        current_root_children,
                    )
                }),
                CommandRedirectTarget::All => suggest_children(
                    reader,
                    roots,
                    ParsedArguments::default(),
                    Vec::new(),
                    context,
                    roots,
                    None,
                ),
            };
        }

        suggest_children(
            reader,
            &self.children,
            arguments,
            dynamic_permissions,
            context,
            roots,
            current_root_children,
        )
    }
}

fn parse_argument(
    parser: &dyn CommandArgumentParser,
    argument_name: &str,
    reader: &mut CommandReader<'_>,
    context: &dyn CommandInputContext,
) -> Result<ParsedArgument, CommandParseError> {
    let cursor = reader.absolute_cursor();
    let value = parser.parse(reader, context)?;
    if reader.absolute_cursor() == cursor {
        return Err(CommandParseError::new(
            CommandParseErrorKind::ArgumentParserDidNotConsumeInput {
                argument: argument_name.to_owned(),
                parsed_type: parser.parsed_type(),
            },
            cursor,
        ));
    }
    Ok(value)
}
