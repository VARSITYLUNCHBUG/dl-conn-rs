//! The pages under the dropdown. Each shows what a command returned, and hands back any command line
//! to run. Working out text and styling is pure, so it can be tested without a window.

use super::theme::{self, Palette};
use crate::command;
use crate::editor::{Action, Editor, Key as EditorKey};
use crate::log;
use crate::session::{Handle, Server, Status};
use crate::settings::{Kind, Row};
use eframe::egui::{
    self, Color32, Event, EventFilter, FontFamily, FontId, Id, Key, Label, Modifiers, RichText,
    ScrollArea, TextEdit, TextFormat, TextStyle, TextWrapMode, Ui, Vec2,
    text::{CCursor, CCursorRange, LayoutJob},
};
use std::collections::{HashMap, VecDeque};

const MONO: f32 = 13.0;
const LABEL_WIDTH: f32 = 210.0;
const PROMPT: &str = "dl-conn-rs>";
/// How much history the console page keeps on screen.
const CONSOLE_LINES: usize = 20_000;

/// What a line of report or help text is, for styling.
#[derive(Debug, PartialEq)]
pub enum LineKind {
    Heading,
    /// A finding, with its marker's length in bytes after the two-space indent.
    Marked(Mark),
    Plain,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mark {
    Ok,
    Check,
    Note,
}

/// Headings are all capitals at the start of a line. Findings start with an indented marker.
pub fn classify(line: &str) -> LineKind {
    let is_heading = line.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && line.chars().any(|c| c.is_ascii_uppercase())
        && !line.chars().any(|c| c.is_ascii_lowercase());
    if is_heading {
        return LineKind::Heading;
    }
    for (marker, mark) in [
        ("  CHECK  ", Mark::Check),
        ("  OK     ", Mark::Ok),
        ("  NOTE   ", Mark::Note),
    ] {
        if line.starts_with(marker) {
            return LineKind::Marked(mark);
        }
    }
    LineKind::Plain
}

/// Report or help text with headings and finding markers picked out, as one selectable block.
pub fn styled_text(lines: &[String], colours: Palette) -> LayoutJob {
    let plain = TextFormat::simple(FontId::new(MONO, FontFamily::Monospace), colours.text);
    let coloured = |colour: Color32| TextFormat {
        color: colour,
        ..plain.clone()
    };
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    for line in lines {
        match classify(line) {
            LineKind::Heading => job.append(line, 0.0, coloured(colours.accent)),
            LineKind::Marked(mark) => {
                let colour = match mark {
                    Mark::Ok => colours.ok,
                    Mark::Check => colours.check,
                    Mark::Note => colours.note,
                };
                let (marker, rest) = line.split_at(9);
                job.append(marker, 0.0, coloured(colour));
                job.append(rest, 0.0, plain.clone());
            }
            LineKind::Plain => job.append(line, 0.0, plain.clone()),
        }
        job.append("\n", 0.0, plain.clone());
    }
    job
}

fn colours(ui: &Ui) -> Palette {
    theme::palette(ui.visuals().dark_mode)
}

/// Selectable text with both scrollbars, for help and the report.
pub fn text_page(ui: &mut Ui, lines: &[String]) {
    let job = styled_text(lines, colours(ui));
    ScrollArea::both().auto_shrink(false).show(ui, |ui| {
        ui.add(
            Label::new(job)
                .selectable(true)
                .wrap_mode(TextWrapMode::Extend),
        );
    });
}

/// The status page's rows, as label and value.
pub fn status_rows(status: &Status) -> Vec<(&'static str, String)> {
    let mut rows = vec![(
        "Server",
        match (status.pick.as_str(), &status.pick_desc) {
            ("", _) => "None, all servers are open".to_string(),
            (code, Some(desc)) => format!("{desc} ({code})"),
            (code, None) => format!("{code}, not in the current relay list"),
        },
    )];
    if !status.pick.is_empty() {
        rows.push((
            "Backup",
            status
                .backup
                .clone()
                .unwrap_or_else(|| "None, one relay with no failover".to_string()),
        ));
        rows.push((
            "Blocked",
            format!(
                "{} relays across {} sites",
                status.blocked, status.sites_blocked
            ),
        ));
    }
    rows.push((
        "Relay list",
        if status.offline {
            "Offline, retrying".to_string()
        } else if status.loaded {
            "Up to date".to_string()
        } else {
            "Loading".to_string()
        },
    ));
    if let Some(error) = &status.error {
        rows.push(("Error", error.clone()));
    }
    rows
}

pub fn status_page(ui: &mut Ui, status: Option<&Status>) {
    let Some(status) = status else {
        ui.label("Loading...");
        return;
    };
    ui.add_space(4.0);
    egui::Grid::new("status")
        .num_columns(2)
        .spacing([18.0, 8.0])
        .show(ui, |ui| {
            for (label, value) in status_rows(status) {
                ui.colored_label(colours(ui).muted, label);
                ui.add(Label::new(value).selectable(true));
                ui.end_row();
            }
        });
    ui.add_space(14.0);
    ui.colored_label(
        colours(ui).muted,
        "A block lasts until you pick again or restart the PC, even after dl-conn-rs closes.",
    );
    ui.colored_label(
        colours(ui).muted,
        "New here? Press F1 for how to test your connection.",
    );
}

/// Whatever the settings page has been asked to do.
#[derive(Default)]
pub struct SettingsPage {
    /// Text being typed into number and path boxes.
    edits: HashMap<&'static str, String>,
    pub message: String,
}

impl SettingsPage {
    /// Drops typed text, so a box goes back to what the session really has.
    pub fn reset_edits(&mut self) {
        self.edits.clear();
    }

    /// Draws the form. Returns a command line to run.
    pub fn show(&mut self, ui: &mut Ui, rows: &[Row], servers: &[Server]) -> Option<String> {
        let palette = colours(ui);
        let mut command = None;
        ui.colored_label(
            palette.muted,
            "None of these lowers ping beyond choosing a route. Changes save straight away.",
        );
        ui.add_space(6.0);
        let footer = ui.spacing().interact_size.y + 44.0;
        ScrollArea::vertical()
            .auto_shrink(false)
            .max_height((ui.available_height() - footer).max(60.0))
            .show(ui, |ui| {
                for row in rows {
                    ui.horizontal(|ui| {
                        let label = ui
                            .allocate_ui(
                                Vec2::new(LABEL_WIDTH, ui.spacing().interact_size.y),
                                |ui| {
                                    ui.set_min_width(LABEL_WIDTH);
                                    ui.strong(row.title)
                                },
                            )
                            .inner;
                        if let Some(value) = self.control(ui, row, servers, &label) {
                            command = Some(format!("set config {} {value}", row.key));
                        }
                    });
                    ui.colored_label(
                        palette.muted,
                        RichText::new(format!("{} Allowed: {}.", row.about, row.allowed)).small(),
                    );
                    ui.add_space(8.0);
                }
            });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Defaults").clicked() {
                command = Some("clear config".to_string());
            }
            ui.colored_label(palette.muted, self.message.as_str());
        });
        command
    }

    /// Draws one setting's control, and returns a new value if it changed.
    fn control(
        &mut self,
        ui: &mut Ui,
        row: &Row,
        servers: &[Server],
        label: &egui::Response,
    ) -> Option<String> {
        match row.kind {
            Kind::Toggle => choice_box(ui, row, &choices(&["on", "off"]), label),
            Kind::Theme => choice_box(ui, row, &choices(&["system", "light", "dark"]), label),
            Kind::Server => choice_box(ui, row, &server_choices(&row.value, servers), label),
            Kind::Number | Kind::Path => {
                let text = self
                    .edits
                    .entry(row.key)
                    .or_insert_with(|| row.value.clone());
                let response = ui
                    .add(TextEdit::singleline(text).desired_width(f32::INFINITY))
                    .labelled_by(label.id);
                let value = text.trim().to_string();
                (response.lost_focus() && value != row.value).then_some(value)
            }
        }
    }
}

fn choices(values: &[&str]) -> Vec<(String, String)> {
    values
        .iter()
        .map(|value| ((*value).to_string(), (*value).to_string()))
        .collect()
}

/// What the backup list offers: none, each server, and a saved one the list no longer has, so it
/// never reads as something else.
pub fn server_choices(value: &str, servers: &[Server]) -> Vec<(String, String)> {
    let mut lines = vec![("none".to_string(), "none (strict)".to_string())];
    lines.extend(servers.iter().map(|server| {
        (
            server.code.clone(),
            format!("{} - {}", server.code, server.desc),
        )
    }));
    if !lines.iter().any(|(code, _)| code == value) {
        lines.push((value.to_string(), value.to_string()));
    }
    lines
}

fn choice_box(
    ui: &mut Ui,
    row: &Row,
    lines: &[(String, String)],
    label: &egui::Response,
) -> Option<String> {
    let shown = lines
        .iter()
        .find(|(value, _)| *value == row.value)
        .map_or(row.value.as_str(), |(_, label)| label.as_str());
    let mut chosen = None;
    let shown_box = egui::ComboBox::from_id_salt(row.key)
        .selected_text(shown)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for (value, label) in lines {
                if ui.selectable_label(*value == row.value, label).clicked() && *value != row.value
                {
                    chosen = Some(value.clone());
                }
            }
        });
    shown_box.response.labelled_by(label.id);
    chosen
}

/// The in-app console: the live log above a prompt with completion and history.
#[derive(Default)]
pub struct ConsolePage {
    lines: VecDeque<String>,
    editor: Editor,
}

impl ConsolePage {
    /// Takes new log text, including everything from before the page first opened.
    fn drain(&mut self) {
        let (clear, text) = log::drain();
        if clear {
            self.lines.clear();
        }
        self.lines.extend(text.lines().map(str::to_string));
        while self.lines.len() > CONSOLE_LINES {
            self.lines.pop_front();
        }
    }

    pub fn show(&mut self, ui: &mut Ui, session: &Handle, can_focus: bool) {
        self.drain();
        egui::Panel::bottom("console-prompt").show(ui, |ui| {
            ui.add_space(4.0);
            self.prompt(ui, session, can_focus);
            ui.add_space(2.0);
        });
        let font = FontId::new(MONO, FontFamily::Monospace);
        let height = ui.fonts_mut(|fonts| fonts.row_height(&font));
        ScrollArea::both()
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show_rows(ui, height, self.lines.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for at in range {
                    ui.add(
                        Label::new(RichText::new(&self.lines[at]).monospace())
                            .selectable(true)
                            .wrap_mode(TextWrapMode::Extend),
                    );
                }
            });
    }

    /// Enter, Tab, the arrows and `?`, taken before the text box can see them.
    fn keys(&mut self, ui: &Ui, id: Id, session: &Handle) {
        let before = (self.editor.line().to_string(), self.editor.cursor());
        let pressed = |key| ui.input_mut(|input| input.consume_key(Modifiers::NONE, key));
        for (key, mapped) in [
            (Key::Enter, EditorKey::Enter),
            (Key::ArrowUp, EditorKey::Up),
            (Key::ArrowDown, EditorKey::Down),
            (Key::Escape, EditorKey::Cancel),
        ] {
            if pressed(key) {
                perform(self.editor.key(mapped), session);
            }
        }
        if pressed(Key::Tab) {
            perform(self.editor.tab(&session.servers()), session);
        }
        let question = ui.input_mut(|input| {
            let is_question = |event: &Event| matches!(event, Event::Text(text) if text == "?");
            let asked = input.events.iter().any(is_question);
            input.events.retain(|event| !is_question(event));
            asked
        });
        if question {
            command::help_for(self.editor.line())
                .into_iter()
                .for_each(log::line);
        }
        if before != (self.editor.line().to_string(), self.editor.cursor()) {
            let mut state = TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
            let caret = CCursor::new(self.editor.cursor());
            state.cursor.set_char_range(Some(CCursorRange::one(caret)));
            state.store(ui.ctx(), id);
        }
    }

    fn prompt(&mut self, ui: &mut Ui, session: &Handle, can_focus: bool) {
        let id = ui.scope_id().with("console-input");
        if ui.memory(|memory| memory.has_focus(id)) {
            if let Some(state) = TextEdit::load_state(ui.ctx(), id)
                && let Some(range) = state.cursor.char_range()
            {
                let line = self.editor.line().to_string();
                self.editor.set_input(&line, range.primary.index.into());
            }
            self.keys(ui, id, session);
        }
        let mut text = self.editor.line().to_string();
        ui.horizontal(|ui| {
            let name = ui.label(RichText::new(PROMPT).monospace());
            let response = ui
                .add(
                    TextEdit::singleline(&mut text)
                        .id(id)
                        .font(TextStyle::Monospace)
                        .hint_text("Type a command, or help")
                        .lock_focus(true)
                        .return_key(None)
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(name.id);
            if can_focus {
                response.request_focus();
            }
            if response.has_focus() {
                // Without this, egui takes focus away on Esc before the line can be cleared.
                ui.memory_mut(|memory| {
                    memory.set_focus_lock_filter(
                        id,
                        EventFilter {
                            tab: true,
                            horizontal_arrows: true,
                            vertical_arrows: true,
                            escape: true,
                        },
                    );
                });
            }
            if response.changed() {
                let caret = TextEdit::load_state(ui.ctx(), id)
                    .and_then(|state| state.cursor.char_range())
                    .map_or(text.len(), |range| range.primary.index.into());
                self.editor.set_input(&text, caret);
            }
        });
    }

    pub fn has_input(&self) -> bool {
        !self.editor.line().is_empty()
    }
}

fn perform(action: Action, session: &Handle) {
    match action {
        Action::None | Action::Redraw => {}
        Action::Submit(line) => {
            // The result reaches the log, so the reply isn't needed here.
            let _ = session.run(&line);
        }
        Action::ShowCandidates(candidates) => {
            let width = candidates.iter().map(|c| c.text.len()).max().unwrap_or(0);
            for candidate in candidates {
                log!("  {:<width$}  {}", candidate.text, candidate.note);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::Monitor;

    fn status() -> Status {
        Status {
            pick: String::new(),
            pick_desc: None,
            backup: None,
            blocked: 0,
            sites_blocked: 0,
            loaded: true,
            offline: false,
            error: None,
            monitor: Monitor::default(),
        }
    }

    #[test]
    fn classifies_headings_and_findings() {
        assert_eq!(classify("FINDINGS"), LineKind::Heading);
        assert_eq!(classify("PING TABLE AT 18:46:31"), LineKind::Heading);
        assert_eq!(classify("STEP 1: TURN ON LOGGING"), LineKind::Heading);
        assert_eq!(classify("dl-conn-rs log report"), LineKind::Plain);
        assert_eq!(classify("Direct"), LineKind::Plain);
        assert_eq!(classify("  F1    This help"), LineKind::Plain);
        assert_eq!(
            classify("  CHECK  Primary relay"),
            LineKind::Marked(Mark::Check)
        );
        assert_eq!(classify("  OK     Fine"), LineKind::Marked(Mark::Ok));
        assert_eq!(
            classify("  NOTE   Worth knowing"),
            LineKind::Marked(Mark::Note)
        );
        assert_eq!(classify("  OK     This was"), LineKind::Marked(Mark::Ok));
        assert_eq!(classify(""), LineKind::Plain);
    }

    #[test]
    fn styled_text_keeps_every_line() {
        let lines = vec![
            "FINDINGS".to_string(),
            "  CHECK  Something".to_string(),
            "plain".to_string(),
        ];
        let job = styled_text(&lines, theme::palette(true));
        assert_eq!(job.text, "FINDINGS\n  CHECK  Something\nplain\n");
    }

    #[test]
    fn status_rows_say_what_is_on() {
        let rows = status_rows(&status());
        assert!(rows[0].1.contains("all servers are open"));
        assert!(!rows.iter().any(|(label, _)| *label == "Blocked"));
        let picked = Status {
            pick: "jnb".to_string(),
            pick_desc: Some("Johannesburg (South Africa)".to_string()),
            blocked: 216,
            sites_blocked: 44,
            ..status()
        };
        let rows = status_rows(&picked);
        assert_eq!(rows[0].1, "Johannesburg (South Africa) (jnb)");
        assert!(
            rows.iter()
                .any(|(_, value)| value == "216 relays across 44 sites")
        );
        assert!(rows.iter().any(|(_, value)| value.contains("no failover")));
    }

    #[test]
    fn status_rows_cover_failures() {
        let rows = status_rows(&Status {
            offline: true,
            error: Some("denied".to_string()),
            ..status()
        });
        assert!(rows.iter().any(|(_, value)| value.contains("Offline")));
        assert!(
            rows.iter()
                .any(|(label, value)| *label == "Error" && value == "denied")
        );
    }

    #[test]
    fn a_backup_list_keeps_a_saved_server_visible() {
        let servers = vec![Server {
            code: "maa2".to_string(),
            desc: "Chennai".to_string(),
            relays: 2,
        }];
        let lines = server_choices("none", &servers);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].0, "none");
        assert_eq!(lines[1].1, "maa2 - Chennai");
        assert_eq!(server_choices("gone", &servers).last().unwrap().0, "gone");
    }
}
