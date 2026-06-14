//! Pixel-only approval for unmask requests.

use serde::{Deserialize, Serialize};
#[cfg(feature = "approval-dialog")]
use stencil_core::Signature;
use stencil_core::{UnmaskApprovalContext, UnmaskApprovalDecision};

#[cfg(feature = "approval-dialog")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "approval-dialog")]
use anyhow::{Result, anyhow};
#[cfg(feature = "approval-dialog")]
use iced::widget::text::Wrapping;
#[cfg(feature = "approval-dialog")]
use iced::widget::{
    Column, button, column, container, horizontal_rule, image, row, scrollable, text, text_editor,
};
#[cfg(feature = "approval-dialog")]
use iced::{Alignment, ContentFit, Element, Length, Size, Task};

#[cfg(feature = "approval-dialog")]
const APPROVAL_ICON: &[u8] = include_bytes!("../assets/approval-icon.png");

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct UnmaskedSnippets {
    context: UnmaskApprovalContext,
    selector: String,
    proposed_name: Option<String>,
    snippets: Vec<RawSnippet>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RawSnippet {
    element_index: usize,
    text: String,
}

impl UnmaskedSnippets {
    pub(crate) fn new(
        context: UnmaskApprovalContext,
        selector: String,
        proposed_name: Option<String>,
        snippets: Vec<RawSnippet>,
    ) -> Self {
        Self {
            context,
            selector,
            proposed_name,
            snippets,
        }
    }
}

impl RawSnippet {
    pub(crate) fn new(element_index: usize, text: String) -> Self {
        Self {
            element_index,
            text,
        }
    }

    #[cfg(feature = "raw")]
    pub fn element_index(&self) -> usize {
        self.element_index
    }

    #[cfg(feature = "raw")]
    pub fn text(&self) -> &str {
        &self.text
    }
}

#[cfg(feature = "approval-dialog")]
pub(crate) fn approve(snippets: UnmaskedSnippets) -> Result<UnmaskApprovalDecision> {
    let result = Arc::new(Mutex::new(UnmaskApprovalDecision::default()));
    let result_for_app = result.clone();
    iced::application("Stencilwright unmask request", update, view)
        .centered()
        .window_size(Size::new(860.0, 820.0))
        .run_with(move || {
            (
                ApprovalApp {
                    snippets,
                    feedback: text_editor::Content::new(),
                    result: result_for_app,
                },
                Task::none(),
            )
        })
        .map_err(|e| anyhow!("approval dialog failed: {e}"))?;
    let decision = result
        .lock()
        .map_err(|_| anyhow!("approval dialog decision mutex poisoned"))?
        .clone();
    Ok(decision)
}

#[cfg(feature = "approval-dialog")]
struct ApprovalApp {
    snippets: UnmaskedSnippets,
    feedback: text_editor::Content,
    result: Arc<Mutex<UnmaskApprovalDecision>>,
}

#[cfg(feature = "approval-dialog")]
#[derive(Debug, Clone)]
enum Message {
    Approve,
    Deny,
    FeedbackChanged(text_editor::Action),
}

#[cfg(feature = "approval-dialog")]
fn update(state: &mut ApprovalApp, message: Message) -> Task<Message> {
    match message {
        Message::Approve => {
            return finish(state, true);
        }
        Message::Deny => {
            return finish(state, false);
        }
        Message::FeedbackChanged(action) => {
            state.feedback.perform(action);
            return Task::none();
        }
    }
}

#[cfg(feature = "approval-dialog")]
fn finish(state: &ApprovalApp, approved: bool) -> Task<Message> {
    if let Ok(mut decision) = state.result.lock() {
        *decision = UnmaskApprovalDecision::new(approved, state.feedback.text());
    }
    iced::exit()
}

#[cfg(feature = "approval-dialog")]
fn view(state: &ApprovalApp) -> Element<'_, Message> {
    let mut snippet_list = Column::new().spacing(10);
    if state.snippets.snippets.is_empty() {
        snippet_list = snippet_list.push(
            container(text("No matching elements found.").size(15))
                .padding(14)
                .width(Length::Fill)
                .style(container::bordered_box),
        );
    } else {
        for snippet in &state.snippets.snippets {
            let raw_text = preview_text(&snippet.text);
            snippet_list = snippet_list.push(
                container(
                    text(raw_text)
                        .size(15)
                        .width(Length::Fill)
                        .wrapping(Wrapping::WordOrGlyph),
                )
                .padding(12)
                .width(Length::Fill)
                .style(container::bordered_box),
            );
        }
    }

    let header = row![
        approval_icon(),
        column![
            text("Stencilwright is requesting to unmask data").size(25),
            text(request_target(&state.snippets))
                .size(17)
                .color([0.10, 0.12, 0.16])
                .width(Length::Fill)
                .wrapping(Wrapping::WordOrGlyph),
        ]
        .spacing(4)
        .width(Length::Fill),
    ]
    .spacing(14)
    .align_y(Alignment::Center);

    let content_heading = format!("Content matches ({})", state.snippets.snippets.len());
    let context = context_panel(&state.snippets);
    let feedback = feedback_panel(&state.feedback);
    let body = scrollable(
        column![
            context,
            horizontal_rule(1),
            column![text(content_heading).size(14).color([0.42, 0.46, 0.54]),].spacing(4),
            snippet_list,
        ]
        .spacing(10),
    )
    .height(Length::Fill);

    let actions = row![
        button("Deny")
            .on_press(Message::Deny)
            .padding([8, 18])
            .style(button::secondary),
        button("Approve")
            .on_press(Message::Approve)
            .padding([8, 18])
            .style(button::primary),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    container(
        column![
            header,
            body,
            horizontal_rule(1),
            feedback,
            container(actions)
                .width(Length::Fill)
                .align_x(iced::alignment::Horizontal::Right),
        ]
        .spacing(12),
    )
    .padding(20)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn preview_text(raw: &str) -> String {
    let mut lines = Vec::new();
    let mut previous_blank = false;
    for line in raw.lines().map(str::trim_end) {
        let line = line.trim_start();
        if line.is_empty() {
            if !previous_blank && !lines.is_empty() {
                lines.push(String::new());
            }
            previous_blank = true;
        } else {
            lines.push(line.to_string());
            previous_blank = false;
        }
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    let text = lines.join("\n");
    if text.is_empty() {
        "(empty text)".to_string()
    } else {
        text
    }
}

#[cfg(feature = "approval-dialog")]
fn request_target(snippets: &UnmaskedSnippets) -> String {
    let context = &snippets.context;
    let mut parts = Vec::new();
    if let Some(site) = context.site.as_deref().filter(|s| !s.is_empty()) {
        parts.push(site.to_string());
    }
    if let Some(place) = context.place.as_deref().filter(|s| !s.is_empty()) {
        parts.push(place.to_string());
    }
    if let Some(name) = snippets.proposed_name.as_deref().filter(|s| !s.is_empty()) {
        parts.push(name.to_string());
    }
    if parts.is_empty() {
        snippets.selector.clone()
    } else {
        parts.join(".")
    }
}

#[cfg(feature = "approval-dialog")]
fn feedback_panel(feedback: &text_editor::Content) -> Element<'_, Message> {
    container(
        column![
            text("Feedback to agent").size(14).color([0.42, 0.46, 0.54]),
            text_editor(feedback)
                .placeholder("Optional guidance. Do not type exact private values.")
                .on_action(Message::FeedbackChanged)
                .padding(10)
                .size(14)
                .wrapping(Wrapping::WordOrGlyph)
                .height(Length::Fixed(64.0)),
        ]
        .spacing(5),
    )
    .width(Length::Fill)
    .into()
}

#[cfg(feature = "approval-dialog")]
fn approval_icon<'a>() -> Element<'a, Message> {
    let icon = image(image::Handle::from_bytes(APPROVAL_ICON))
        .width(Length::Fixed(56.0))
        .height(Length::Fixed(56.0))
        .content_fit(ContentFit::Cover);

    container(icon)
        .width(Length::Fixed(56.0))
        .height(Length::Fixed(56.0))
        .clip(true)
        .style(container::rounded_box)
        .into()
}

#[cfg(feature = "approval-dialog")]
fn context_panel(snippets: &UnmaskedSnippets) -> Element<'_, Message> {
    let context = &snippets.context;
    let element_name = snippets.proposed_name.as_deref().unwrap_or("(unchanged)");

    let mut details = Column::new()
        .spacing(5)
        .push(detail_row(
            "scope",
            context.scope.as_deref().unwrap_or("(unspecified)"),
        ))
        .push(detail_row(
            "site",
            context.site.as_deref().unwrap_or("(unknown)"),
        ))
        .push(detail_row(
            "place",
            context.place.as_deref().unwrap_or("(site-wide)"),
        ))
        .push(detail_row(
            "current URL",
            context.current_url.as_deref().unwrap_or("(unknown)"),
        ))
        .push(detail_row(
            "request reason",
            context.reason.as_deref().unwrap_or("(not provided)"),
        ))
        .push(detail_row("selector", &snippets.selector))
        .push(detail_row("element name", element_name));

    if let Some(signature) = &context.signature {
        details = details.push(signature_rows(signature));
    } else {
        details = details.push(detail_row("matching criteria", "(no place signature)"));
    }

    container(details)
        .padding(10)
        .width(Length::Fill)
        .style(container::rounded_box)
        .into()
}

#[cfg(feature = "approval-dialog")]
fn signature_rows(signature: &Signature) -> Element<'_, Message> {
    let mut rows = Column::new().spacing(8);
    let mut has_signature_fields = false;
    if let Some(url) = &signature.url {
        rows = rows.push(detail_row("signature.url", url));
        has_signature_fields = true;
    }
    if let Some(selector) = &signature.selector {
        rows = rows.push(detail_row("signature.selector", selector));
        has_signature_fields = true;
    }
    if let Some(selector) = &signature.visible_selector {
        rows = rows.push(detail_row("signature.visible", selector));
        has_signature_fields = true;
    }
    if let Some(selector) = &signature.absent_selector {
        rows = rows.push(detail_row("signature.absent", selector));
        has_signature_fields = true;
    }
    if let Some(text_match) = &signature.text {
        rows = rows.push(detail_row("signature.text", text_match));
        has_signature_fields = true;
    }
    if !has_signature_fields {
        rows = rows.push(detail_row("matching criteria", "(empty signature)"));
    }
    rows.into()
}

#[cfg(feature = "approval-dialog")]
fn detail_row<'a>(label: &'static str, value: impl Into<String>) -> Element<'a, Message> {
    row![
        container(text(label).size(12).color([0.42, 0.46, 0.54])).width(Length::Fixed(112.0)),
        text(value.into())
            .size(12)
            .width(Length::Fill)
            .wrapping(Wrapping::WordOrGlyph),
    ]
    .spacing(10)
    .align_y(Alignment::Start)
    .into()
}

#[cfg(test)]
mod tests {
    use super::preview_text;

    #[test]
    fn preview_text_collapses_excess_blank_lines() {
        assert_eq!(
            preview_text("\n\n  Alpha\n\n\n  Beta  \n\n"),
            "Alpha\n\nBeta"
        );
    }

    #[test]
    fn preview_text_labels_empty_content() {
        assert_eq!(preview_text("  \n\t\n"), "(empty text)");
    }
}
