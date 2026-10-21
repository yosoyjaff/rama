//! The panel for a commit: its message, who wrote it and when, what it
//! changed in total, and its files.

use gpui_kit::{AnyElement, Context, FontWeight, Hsla, SharedString, div, prelude::*, px};

use super::rows::{List, band};
use super::work::Fetched;
use super::{DetailPanel, Tally};
use crate::features::history::history_author_name;
use crate::git::Commit;
use crate::ui::avatar::{author_initials, author_tint};
use crate::ui::theme::{self, Theme};

/// When a commit was written, in local time, as `04/03/2026 @ 10:15`; a
/// dash if git's date does not parse.
fn written_at(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339).map_or_else(
        |_| "—".to_owned(),
        |date| {
            date.with_timezone(&chrono::Local)
                .format("%d/%m/%Y @ %H:%M")
                .to_string()
        },
    )
}

/// A commit body as points: one per non-blank line, without the `-` or `*`
/// the author may have used for one.
fn points(body: &str) -> impl Iterator<Item = &str> {
    body.lines()
        .map(|line| line.trim().trim_start_matches(['-', '*']).trim_start())
        .filter(|line| !line.is_empty())
}

impl DetailPanel {
    pub(super) fn render_commit(
        &self,
        commit: &Commit,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(message(commit, theme))
            .child(byline(commit, theme))
            .children(self.totals(theme))
            .child(self.listing_bar(theme, cx))
            .child(self.commit_files(theme, cx))
            .into_any_element()
    }

    /// "3 modified  1 added", each with its letter in its colour.
    fn totals(&self, theme: &Theme) -> Option<AnyElement> {
        let files = self.commit.listed();
        if files.is_empty() {
            return None;
        }
        let parts = Tally::of_files(files).present().map(|(change, n)| {
            div()
                .flex()
                .items_center()
                .gap(px(3.0))
                .child(div().text_color(change.color(theme)).child(change.letter()))
                .child(SharedString::from(format!("{n} {}", change.word())))
        });
        Some(
            band(theme)
                .gap(px(10.0))
                .px(px(10.0))
                .py(px(5.0))
                .text_size(px(10.0))
                .text_color(theme.text_muted)
                .children(parts)
                .into_any_element(),
        )
    }

    fn commit_files(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let list = div()
            .id("panel-commit-files")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        let note = |text: SharedString, color: Hsla| {
            div()
                .px(px(10.0))
                .py(px(8.0))
                .text_size(px(11.0))
                .text_color(color)
                .child(text)
        };
        match &self.commit.files {
            Fetched::Waiting => list.child(note("Reading the files…".into(), theme.text_faint)),
            Fetched::Failed(error) => list.child(note(error.clone(), theme.danger_muted)),
            Fetched::Ready(files) if files.is_empty() => list.child(note(
                "This commit changes no files.".into(),
                theme.text_faint,
            )),
            Fetched::Ready(files) => {
                list.py(px(4.0))
                    .children(self.file_rows(List::Commit, files, theme, cx))
            }
        }
        .into_any_element()
    }
}

/// The subject in bold, the body as points under it.
fn message(commit: &Commit, theme: &Theme) -> AnyElement {
    let subject = match commit.subject.trim() {
        "" => SharedString::from("(no subject)"),
        subject => SharedString::from(subject.to_owned()),
    };
    let point = |text: &str| {
        div()
            .flex()
            .items_start()
            .gap(px(6.0))
            .text_size(px(11.0))
            .text_color(theme.text_muted)
            .child(div().flex_none().text_color(theme.text_faint).child("·"))
            .child(div().min_w_0().child(SharedString::from(text.to_owned())))
    };
    div()
        .flex_none()
        .flex()
        .flex_col()
        .gap(px(6.0))
        .px(px(10.0))
        .py(px(9.0))
        .bg(theme::ink(0.03))
        .child(
            div()
                .min_w_0()
                .text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text)
                .child(subject),
        )
        .children(points(&commit.body).map(point))
        .into_any_element()
}

/// The author's initials on their colour, their name and the date, and
/// the commit this one follows.
fn byline(commit: &Commit, theme: &Theme) -> AnyElement {
    let name = history_author_name(&commit.author.name);
    // The colour follows the email, so one person keeps it across names.
    let tint_key = match commit.author.email.trim() {
        "" => name.to_string(),
        email => email.to_owned(),
    };
    let badge = div()
        .flex_none()
        .size(px(24.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.0))
        .bg(author_tint(&tint_key))
        .font_family(theme.font_mono.clone())
        .font_weight(FontWeight::BOLD)
        .text_size(px(10.0))
        .text_color(theme.bg)
        .child(SharedString::from(author_initials(&name)));
    let who = div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(
            div()
                .truncate()
                .text_size(px(11.0))
                .text_color(theme.text)
                .child(name),
        )
        .child(
            div()
                .text_size(px(10.0))
                .text_color(theme.text_faint)
                .child(SharedString::from(format!(
                    "authored {}",
                    written_at(&commit.author.when)
                ))),
        );
    let parent = commit.parents.first().map(|sha| {
        div()
            .flex_none()
            .font_family(theme.font_mono.clone())
            .text_size(px(10.0))
            .text_color(theme.text_faint)
            .child(SharedString::from(format!(
                "parent {}",
                sha.get(..7).unwrap_or(sha)
            )))
    });
    band(theme)
        .gap(px(8.0))
        .px(px(10.0))
        .py(px(6.0))
        .child(badge)
        .child(who)
        .children(parent)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_reads_as_points() {
        let body = "- one\n\n  * two\nthree\n   ";
        assert_eq!(points(body).collect::<Vec<_>>(), ["one", "two", "three"]);
        assert_eq!(points("").count(), 0);
    }

    #[test]
    fn dates_are_local_and_bad_ones_a_dash() {
        assert_eq!(written_at("yesterday"), "—");
        let written = written_at("2026-03-04T10:15:00+00:00");
        assert!(written.contains("/2026 @ "), "{written}");
    }
}
