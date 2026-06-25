//! Module defining errors that can occur during command execution.
use steel_utils::translations;
use text_components::{Modifier, TextComponent, format::Color, interactivity::ClickEvent};

const BRIGADIER_CONTEXT_AMOUNT: usize = 10;

/// An error that can occur during command execution.
pub enum CommandError {
    /// This error means that there was an error while parsing a previously consumed argument.
    /// That only happens when consumption is wrongly implemented, as it should ensure parsing may
    /// never fail.
    InvalidConsumption(Option<String>),
    /// Return this if a condition that a [`Node::Require`] should ensure is met is not met.
    InvalidRequirement,
    /// The command could not be executed due to insufficient permissions.
    /// The user attempting to run the command lacks the necessary authorization.
    PermissionDenied,
    /// A structured parse failure that can render the vanilla error context.
    Parse(CommandParseErrorReport),
    /// A general error occurred during command execution that doesn't fit into
    /// more specific `CommandError` variants.
    CommandFailed(Box<TextComponent>),
}

impl CommandError {
    /// Creates a structured command parse error.
    #[must_use]
    pub fn parse(message: TextComponent, input: &str, cursor: usize) -> Self {
        Self::Parse(CommandParseErrorReport::new(message, input, cursor))
    }
}

/// Structured parse diagnostic used to render vanilla-style command failures.
pub struct CommandParseErrorReport {
    message: TextComponent,
    input: String,
    cursor: usize,
}

impl CommandParseErrorReport {
    /// Creates a parse diagnostic for `input` at `cursor`.
    #[must_use]
    pub fn new(message: TextComponent, input: &str, cursor: usize) -> Self {
        Self {
            message,
            input: input.to_owned(),
            cursor,
        }
    }

    pub(crate) fn into_feedback(self) -> CommandErrorFeedback {
        let context = Some(self.context_component());
        CommandErrorFeedback {
            primary: self.message,
            context,
        }
    }

    fn context_component(&self) -> TextComponent {
        let cursor = self.cursor.min(self.input.len());
        let cursor = floor_char_boundary(&self.input, cursor);
        let start = context_start(&self.input, cursor);

        let mut context = TextComponent::new()
            .color(Color::Gray)
            .click_event(ClickEvent::suggest_command(self.suggested_command()));

        if start > 0 {
            context = context.add_child(TextComponent::const_plain("..."));
        }

        context = context.add_child(TextComponent::plain(self.input[start..cursor].to_owned()));

        if cursor < self.input.len() {
            context = context.add_child(
                TextComponent::plain(self.input[cursor..].to_owned())
                    .color(Color::Red)
                    .underlined(true),
            );
        }

        context = context.add_child(
            TextComponent::from(&translations::COMMAND_CONTEXT_HERE)
                .color(Color::Red)
                .italic(true),
        );

        context
    }

    fn suggested_command(&self) -> String {
        if self.input.starts_with('/') {
            self.input.clone()
        } else {
            format!("/{}", self.input)
        }
    }
}

/// Rendered command failure messages.
pub(crate) struct CommandErrorFeedback {
    pub(crate) primary: TextComponent,
    pub(crate) context: Option<TextComponent>,
}

impl CommandErrorFeedback {
    pub(crate) const fn single(primary: TextComponent) -> Self {
        Self {
            primary,
            context: None,
        }
    }
}

fn context_start(input: &str, cursor: usize) -> usize {
    input[..cursor]
        .char_indices()
        .rev()
        .nth(BRIGADIER_CONTEXT_AMOUNT)
        .map_or(0, |(index, _)| {
            index + input[index..].chars().next().map_or(0, char::len_utf8)
        })
}

fn floor_char_boundary(input: &str, cursor: usize) -> usize {
    if input.is_char_boundary(cursor) {
        return cursor;
    }

    let mut cursor = cursor;
    while cursor > 0 && !input.is_char_boundary(cursor) {
        cursor -= 1;
    }
    cursor
}

#[cfg(test)]
mod tests {
    use super::CommandParseErrorReport;
    use steel_utils::translations;
    use text_components::{
        TextComponent, content::Content, format::Color, interactivity::ClickEvent,
    };

    fn text_content(component: &TextComponent) -> &str {
        let Content::Text { text } = &component.content else {
            panic!("component should be plain text");
        };
        text
    }

    #[test]
    fn parse_context_uses_brigadier_window_and_remaining_input() {
        let report = CommandParseErrorReport::new(
            TextComponent::from(&translations::COMMAND_UNKNOWN_COMMAND),
            "0123456789abcdef",
            13,
        );
        let feedback = report.into_feedback();
        let context = feedback.context.expect("parse feedback includes context");

        assert_eq!(context.format.color, Some(Color::Gray));
        assert!(matches!(
            context.interactions.click,
            Some(ClickEvent::SuggestCommand { ref command })
                if command.as_ref() == "/0123456789abcdef"
        ));
        assert_eq!(context.children.len(), 4);
        assert_eq!(text_content(&context.children[0]), "...");
        assert_eq!(text_content(&context.children[1]), "3456789abc");
        assert_eq!(text_content(&context.children[2]), "def");
        assert_eq!(context.children[2].format.color, Some(Color::Red));
        assert_eq!(context.children[2].format.underlined, Some(true));
        assert!(matches!(
            &context.children[3].content,
            Content::Translate(message)
                if message.key.as_ref() == translations::COMMAND_CONTEXT_HERE.0
        ));
        assert_eq!(context.children[3].format.color, Some(Color::Red));
        assert_eq!(context.children[3].format.italic, Some(true));
    }

    #[test]
    fn parse_context_uses_utf8_char_boundaries() {
        let report = CommandParseErrorReport::new(
            TextComponent::from(&translations::COMMAND_UNKNOWN_COMMAND),
            "say 🙂 tail",
            "say 🙂".len() - 1,
        );
        let feedback = report.into_feedback();
        let context = feedback.context.expect("parse feedback includes context");

        assert_eq!(text_content(&context.children[0]), "say ");
    }
}
