//! The searchable server dropdown: a text box over a popup of rows grouped by region. What's listed
//! is decided in `rows`. Choosing a row hands back a command line and does nothing itself.

use super::app::ViewData;
use super::rows::{self, Row};
use super::theme;
use eframe::egui::{
    self, Align2, Id, Key, Modifiers, PopupCloseBehavior, Pos2, Rect, Role, ScrollArea, Sense,
    Stroke, TextEdit, TextStyle, Ui, Vec2, WidgetInfo,
};

const ROW_HEIGHT: f32 = 26.0;
const HEADER_HEIGHT: f32 = 24.0;
const INDENT: f32 = 14.0;
const CHEVRON_WIDTH: f32 = 26.0;
const LIST_MARGIN: f32 = 24.0;

#[derive(Default)]
pub struct Picker {
    open: bool,
    query: String,
    highlight: Option<usize>,
    /// The code the dropdown last showed, so choosing it again doesn't issue a command.
    selected: String,
    /// Scroll the highlighted row into view on the next frame.
    reveal: bool,
    /// Whether the window was active last frame, so only leaving it closes the list.
    was_active: bool,
}

impl Picker {
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Takes the session's pick, unless someone is using the list.
    pub fn sync(&mut self, pick: &str) {
        if !self.open {
            pick.clone_into(&mut self.selected);
        }
    }

    fn open_list(&mut self, rows: &[Row]) {
        self.open = true;
        self.query.clear();
        self.highlight = rows::position(rows, &self.selected);
        self.reveal = true;
    }

    fn close(&mut self, ui: &Ui, id: Id) {
        self.open = false;
        self.query.clear();
        self.highlight = None;
        ui.memory_mut(|memory| memory.surrender_focus(id));
    }

    /// Closes the list and returns the command for this server, if it changes anything.
    fn choose(&mut self, ui: &Ui, id: Id, code: &str) -> Option<String> {
        self.close(ui, id);
        if self.selected == code {
            return None;
        }
        code.clone_into(&mut self.selected);
        Some(if code.is_empty() {
            "clear pick".to_string()
        } else {
            format!("set pick {code}")
        })
    }

    /// Arrow keys, Enter and Esc, taken before the text box can see them.
    fn keys(&mut self, ui: &Ui, id: Id, rows: &[Row]) -> Option<String> {
        let pressed = |key| ui.input_mut(|input| input.consume_key(Modifiers::NONE, key));
        if pressed(Key::Escape) {
            self.close(ui, id);
            return None;
        }
        if pressed(Key::ArrowDown) {
            self.highlight = rows::step(rows, self.highlight, true);
            self.reveal = true;
        }
        if pressed(Key::ArrowUp) {
            self.highlight = rows::step(rows, self.highlight, false);
            self.reveal = true;
        }
        if pressed(Key::Enter) {
            let code = self
                .highlight
                .and_then(|at| rows.get(at))
                .or_else(|| rows.iter().find(|row| !row.is_header()))
                .and_then(|row| row.code.clone());
            return code.and_then(|code| self.choose(ui, id, &code));
        }
        None
    }

    /// Draws the box and, while open, the list. Returns a command line to run.
    pub fn show(&mut self, ui: &mut Ui, data: Option<&ViewData>) -> Option<String> {
        let id = ui.scope_id().with("server-picker");
        let Some(data) = data else {
            ui.horizontal(|ui| {
                let name = ui.label("Server");
                ui.add_enabled(
                    false,
                    TextEdit::singleline(&mut "Loading the relay list...".to_string())
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(name.id);
            });
            return None;
        };
        let query = if self.open { self.query.as_str() } else { "" };
        let mut rows = rows::build_rows(&data.servers, &data.status, query);
        let mut command = None;
        if self.open {
            command = self.keys(ui, id, &rows);
        }

        let full = rows::build_rows(&data.servers, &data.status, "");
        let current = rows::position(&full, &self.selected)
            .map_or_else(String::new, |at| full[at].label.clone());
        let mut text = if self.open {
            self.query.clone()
        } else {
            current.clone()
        };

        let (edit, chevron) = ui
            .horizontal(|ui| {
                let name = ui.label("Server");
                let width = ui.available_width() - CHEVRON_WIDTH - ui.spacing().item_spacing.x;
                let edit = ui.add_sized(
                    [width, ui.spacing().interact_size.y],
                    TextEdit::singleline(&mut text)
                        .id(id)
                        .hint_text(current.as_str())
                        .margin(Vec2::new(8.0, 4.0)),
                );
                let edit = edit.labelled_by(name.id);
                let chevron = self.chevron(ui);
                (edit, chevron)
            })
            .inner;

        // Text can arrive with no focus, from dictation or a screen reader, so this opens the list too.
        if edit.changed() {
            self.open = true;
            self.query.clone_from(&text);
            rows = rows::build_rows(&data.servers, &data.status, &self.query);
            self.highlight = rows::first_match(&rows, &self.query);
            self.reveal = true;
        }
        if chevron.clicked() {
            if self.open {
                self.close(ui, id);
            } else {
                self.open_list(&full);
                rows = full;
                ui.memory_mut(|memory| memory.request_focus(id));
            }
        } else if !self.open && edit.has_focus() {
            self.open_list(&full);
            rows = full;
        }
        let window_inactive = ui.input(|input| input.viewport().focused) == Some(false);
        if self.open && window_inactive && self.was_active {
            self.close(ui, id);
        }
        self.was_active = !window_inactive;
        if self.open {
            // Clicking the list or its scrollbar shouldn't stop typing from reaching the box.
            ui.memory_mut(|memory| memory.request_focus(id));
            let (over_popup, chosen) = self.popup(ui, &edit, chevron.rect, &rows);
            if let Some(code) = chosen {
                command = self.choose(ui, id, &code);
            } else if clicked_away(ui, &edit, &chevron, over_popup) {
                self.close(ui, id);
            }
        }
        command
    }

    fn chevron(&self, ui: &mut Ui) -> egui::Response {
        let size = Vec2::new(CHEVRON_WIDTH, ui.spacing().interact_size.y);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        response.widget_info(|| {
            WidgetInfo::labeled(Role::Button, ui.is_enabled(), "Open the server list")
        });
        let visuals = ui.style().interact(&response);
        ui.painter()
            .rect_filled(rect, visuals.corner_radius, visuals.bg_fill);
        let center = rect.center();
        let (x, y) = if self.open { (5.0, -3.0) } else { (5.0, 3.0) };
        ui.painter().add(egui::Shape::convex_polygon(
            vec![
                Pos2::new(center.x - x, center.y - y),
                Pos2::new(center.x + x, center.y - y),
                Pos2::new(center.x, center.y + y),
            ],
            visuals.fg_stroke.color,
            Stroke::NONE,
        ));
        response
    }

    /// Draws the list under the box. Returns whether the pointer is over it, and any row clicked.
    fn popup(
        &mut self,
        ui: &Ui,
        edit: &egui::Response,
        chevron: Rect,
        rows: &[Row],
    ) -> (bool, Option<String>) {
        let width = chevron.right() - edit.rect.left();
        let room = ui.ctx().content_rect().bottom() - edit.rect.bottom() - LIST_MARGIN;
        let mut chosen = None;
        let shown = egui::Popup::from_response(edit)
            .open(true)
            .close_behavior(PopupCloseBehavior::IgnoreClicks)
            .width(width)
            .show(|ui| {
                ui.set_width(width - 2.0 * ui.spacing().window_margin.leftf());
                if rows.is_empty() {
                    let muted = theme::palette(ui.visuals().dark_mode).muted;
                    ui.colored_label(muted, "No matching servers");
                    return;
                }
                ScrollArea::vertical()
                    .max_height(room.max(HEADER_HEIGHT))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (index, row) in rows.iter().enumerate() {
                            if let Some(code) = self.row(ui, index, row) {
                                chosen = Some(code);
                            }
                        }
                    });
            });
        self.reveal = false;
        let over = shown.is_some_and(|inner| {
            ui.input(|input| input.pointer.interact_pos())
                .is_some_and(|at| inner.response.rect.contains(at))
        });
        (over, chosen)
    }

    /// Draws one row, and returns its code if it was clicked.
    fn row(&mut self, ui: &mut Ui, index: usize, row: &Row) -> Option<String> {
        let colours = theme::palette(ui.visuals().dark_mode);
        let width = ui.available_width();
        let font = TextStyle::Body.resolve(ui.style());
        if row.is_header() {
            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(width, HEADER_HEIGHT), Sense::hover());
            response.widget_info(|| WidgetInfo::labeled(Role::Label, true, &row.label));
            ui.painter().text(
                rect.left_center() + Vec2::new(4.0, 0.0),
                Align2::LEFT_CENTER,
                &row.label,
                TextStyle::Small.resolve(ui.style()),
                colours.muted,
            );
            return None;
        }
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, ROW_HEIGHT), Sense::click());
        let highlighted = self.highlight == Some(index);
        response.widget_info(|| {
            WidgetInfo::selected(Role::ListBoxOption, true, highlighted, &row.label)
        });
        if response.hovered() && ui.input(|input| input.pointer.delta() != Vec2::ZERO) {
            self.highlight = Some(index);
        }
        if highlighted {
            ui.painter().rect_filled(rect, 4.0, colours.accent);
        }
        if highlighted && self.reveal {
            ui.scroll_to_rect(rect, None);
        }
        let text = if highlighted {
            colours.on_accent
        } else {
            colours.text
        };
        ui.painter().text(
            rect.left_center() + Vec2::new(INDENT, 0.0),
            Align2::LEFT_CENTER,
            &row.label,
            font,
            text,
        );
        response.clicked().then(|| row.code.clone()).flatten()
    }
}

fn clicked_away(ui: &Ui, edit: &egui::Response, chevron: &egui::Response, over: bool) -> bool {
    !over && !edit.hovered() && !chevron.hovered() && ui.input(|input| input.pointer.any_pressed())
}
