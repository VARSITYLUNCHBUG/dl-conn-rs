//! The window: dropdown and counters at the top, one page below. It holds no logic of its own: every
//! action is a command line sent to the session, and everything shown comes from querying it.

use super::pages::{self, ConsolePage, SettingsPage};
use super::picker::Picker;
use super::rows::{self, View};
use super::theme;
use crate::log;
use crate::session::{Handle, Host, Output, Reply, Server, Status};
use crate::settings::Row;
use eframe::egui::{self, Context, Key, Modifiers, RichText, Ui, ViewportCommand};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Page {
    Status,
    Help,
    Settings,
    Report,
    Console,
}

const TABS: [(Page, &str); 5] = [
    (Page::Status, "Status"),
    (Page::Help, "Help  F1"),
    (Page::Settings, "Settings  F2"),
    (Page::Report, "Report  F3"),
    (Page::Console, "Console  `"),
];

const DRAW_HIDDEN_OVERRIDE: &str = "DL_CONN_RS_DRAW_HIDDEN";
const THEMES: [(&str, &str); 3] = [("system", "System"), ("light", "Light"), ("dark", "Dark")];

/// Everything the top of the window and the status page show, from one round of queries.
#[derive(Debug, PartialEq)]
pub struct ViewData {
    pub servers: Vec<Server>,
    pub status: Status,
    pub config: Vec<Row>,
    pub view: View,
}

/// Results handed back to the UI thread, which never waits on the session.
pub enum Update {
    View(Box<ViewData>),
    Guide(Vec<String>),
    Report(Vec<String>),
    Applied(String),
    Console(bool),
    Quit,
}

/// What the session needs from this front-end.
pub struct UiHost {
    changed: Sender<()>,
    updates: Sender<Update>,
    console: Arc<AtomicBool>,
    ctx: Context,
}

impl UiHost {
    fn send(&self, update: Update) {
        let _ = self.updates.send(update);
        self.ctx.request_repaint();
    }
}

impl Host for UiHost {
    fn toggle_console(&self) -> bool {
        let visible = !self.console.fetch_not(Ordering::AcqRel);
        self.send(Update::Console(visible));
        visible
    }

    fn quit(&self) {
        self.send(Update::Quit);
    }

    fn changed(&self) {
        let _ = self.changed.send(());
    }
}

pub struct App {
    session: Handle,
    updates: Receiver<Update>,
    sender: Sender<Update>,
    ctx: Context,
    page: Page,
    picker: Picker,
    data: Option<Box<ViewData>>,
    guide: Vec<String>,
    report: Vec<String>,
    report_busy: bool,
    settings: SettingsPage,
    console: ConsolePage,
    theme: String,
    closing: bool,
    /// How many frames were drawn, because a window at rest should draw almost none.
    frames: u64,
    /// Handle input even when hidden, for test scripts, which may not have the window in view.
    draw_hidden: bool,
}

impl App {
    /// Starts a session through `start` and connects the window to it.
    pub fn start(ctx: Context, start: impl FnOnce(UiHost) -> Handle) -> Self {
        let (changed, changes) = channel();
        let (sender, updates) = channel();
        let console = Arc::new(AtomicBool::new(false));
        let session = start(UiHost {
            changed,
            updates: sender.clone(),
            console: console.clone(),
            ctx: ctx.clone(),
        });
        let (repaint, view_session, view_sender) = (ctx.clone(), session.clone(), sender.clone());
        thread::spawn(move || view_loop(&view_session, &changes, &view_sender, &repaint));
        let wake = ctx.clone();
        // New log text only needs drawing while the console is on screen.
        log::set_wake(Box::new(move || {
            if console.load(Ordering::Acquire) {
                wake.request_repaint();
            }
        }));
        Self {
            session,
            updates,
            sender,
            ctx,
            page: Page::Status,
            picker: Picker::default(),
            data: None,
            guide: Vec::new(),
            report: Vec::new(),
            report_busy: false,
            settings: SettingsPage::default(),
            console: ConsolePage::default(),
            theme: String::new(),
            closing: false,
            frames: 0,
            draw_hidden: std::env::var_os(DRAW_HIDDEN_OVERRIDE).is_some(),
        }
    }

    /// Runs a command for its data on a short-lived thread, and hands the result back.
    fn fetch(&self, line: &'static str, finish: fn(Vec<String>) -> Update) {
        let (session, sender, ctx) = (self.session.clone(), self.sender.clone(), self.ctx.clone());
        thread::spawn(move || {
            let lines = match session.query(line).recv() {
                Ok(Ok(output)) => output.render(),
                Ok(Err(error)) => vec![error],
                Err(_) => vec!["The session has closed.".to_string()],
            };
            let _ = sender.send(finish(lines));
            ctx.request_repaint();
        });
    }

    /// Runs a command as the user would, and shows how it went on the settings page.
    fn apply(&self, line: String) {
        let (session, sender, ctx) = (self.session.clone(), self.sender.clone(), self.ctx.clone());
        thread::spawn(move || {
            let message = applied_message(session.run(&line).recv().ok());
            let _ = sender.send(Update::Applied(message));
            ctx.request_repaint();
        });
    }

    fn run(&self, line: &str) {
        // The UI reacts to the "changed" signal rather than to individual replies.
        let _ = self.session.run(line);
    }

    fn take_updates(&mut self, ctx: &Context) {
        while let Ok(update) = self.updates.try_recv() {
            match update {
                Update::View(data) => self.take_view(ctx, data),
                Update::Guide(lines) => self.guide = lines,
                Update::Report(lines) => {
                    self.report = lines;
                    self.report_busy = false;
                }
                Update::Applied(message) => {
                    self.settings.message = message;
                    self.settings.reset_edits();
                }
                Update::Console(true) => self.page = Page::Console,
                Update::Console(false) => {
                    if self.page == Page::Console {
                        self.page = Page::Status;
                    }
                }
                Update::Quit => {
                    self.closing = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
    }

    fn take_view(&mut self, ctx: &Context, data: Box<ViewData>) {
        self.picker.sync(&data.status.pick);
        if self.data.as_ref().map(|old| &old.view.title) != Some(&data.view.title) {
            ctx.send_viewport_cmd(ViewportCommand::Title(data.view.title.clone()));
        }
        let theme = data
            .config
            .iter()
            .find(|row| row.key == "theme")
            .map(|row| row.value.as_str());
        if let Some(theme) = theme
            && theme != self.theme
        {
            self.theme = theme.to_string();
            theme::apply(ctx, theme);
        }
        self.data = Some(data);
    }

    /// Moves to a page. The console has its own command, because its report starts and stops with it.
    fn open(&mut self, page: Page) {
        let was_console = self.page == Page::Console;
        match page {
            Page::Console => {
                if !was_console {
                    self.run("toggle console");
                }
                return;
            }
            Page::Report if !self.report_busy => {
                self.report_busy = true;
                self.report = vec!["Reading the game's log...".to_string()];
                self.fetch("analyze log", Update::Report);
            }
            Page::Help if self.guide.is_empty() => self.fetch("show guide", Update::Guide),
            _ => {}
        }
        if was_console {
            self.run("toggle console");
        }
        self.settings.reset_edits();
        self.page = page;
    }

    /// Filtered first, so these keys work whichever control has focus, and never type into one.
    fn hotkeys(&mut self, ctx: &Context) {
        let pressed = |key| ctx.input_mut(|input| input.consume_key(Modifiers::NONE, key));
        if pressed(Key::F1) {
            self.open(Page::Help);
        }
        if pressed(Key::F2) {
            self.open(Page::Settings);
        }
        if pressed(Key::F3) {
            self.open(Page::Report);
        }
        let backtick = ctx.input_mut(|input| {
            let text = |event: &egui::Event| {
                matches!(event, egui::Event::Text(text) if text == "`" || text == "~")
            };
            let typed = input.events.iter().any(text);
            input.events.retain(|event| !text(event));
            typed | input.consume_key(Modifiers::NONE, Key::Backtick)
        });
        if backtick {
            self.toggle_console_key();
        }
        // An open list and a half-typed command both use Esc for themselves.
        let esc_is_free = !self.picker.is_open()
            && !(self.page == Page::Console && self.console.has_input())
            && self.page != Page::Status;
        if esc_is_free && pressed(Key::Escape) {
            self.open(Page::Status);
        }
    }

    fn toggle_console_key(&mut self) {
        if self.page == Page::Console {
            self.open(Page::Status);
        } else {
            self.open(Page::Console);
        }
    }

    /// Closing is a command, so a setting can act on it before the window goes.
    fn handle_close(&mut self, ctx: &Context) {
        if !self.closing && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.run("quit");
        }
    }

    fn tabs(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            for (page, label) in TABS {
                if ui.selectable_label(self.page == page, label).clicked() && self.page != page {
                    self.open(page);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let current = THEMES
                    .iter()
                    .find(|(value, _)| *value == self.theme)
                    .map_or("System", |(_, label)| *label);
                let shown = egui::ComboBox::from_id_salt("theme")
                    .selected_text(current)
                    .width(84.0)
                    .show_ui(ui, |ui| {
                        for (value, label) in THEMES {
                            if ui.selectable_label(self.theme == value, label).clicked() {
                                self.run(&format!("set config theme {value}"));
                            }
                        }
                    });
                let name = ui.label("Theme");
                shown.response.labelled_by(name.id);
            });
        });
    }

    fn header(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        if let Some(command) = self.picker.show(ui, self.data.as_deref()) {
            self.run(&command);
        }
        ui.add_space(4.0);
        if let Some(data) = &self.data {
            ui.label(RichText::new(&data.view.direct));
            ui.label(RichText::new(&data.view.route));
        }
        ui.add_space(4.0);
        self.tabs(ui);
        ui.add_space(2.0);
    }

    fn body(&mut self, ui: &mut Ui) {
        match self.page {
            Page::Status => {
                pages::status_page(ui, self.data.as_ref().map(|data| &data.status));
            }
            Page::Help => pages::text_page(ui, &self.guide),
            Page::Settings => {
                if let Some(data) = &self.data {
                    let (config, servers) = (data.config.clone(), data.servers.clone());
                    if let Some(line) = self.settings.show(ui, &config, &servers) {
                        self.apply(line);
                    }
                }
            }
            Page::Report => {
                ui.horizontal(|ui| {
                    if ui.button("Re-run").clicked() && !self.report_busy {
                        self.open(Page::Report);
                    }
                    if ui.button("Copy").clicked() {
                        ui.ctx().copy_text(self.report.join("\r\n"));
                    }
                });
                ui.add_space(4.0);
                pages::text_page(ui, &self.report);
            }
            Page::Console => {
                let can_focus = !self.picker.is_open();
                self.console.show(ui, &self.session, can_focus);
            }
        }
    }

    /// What must keep working while the window is minimized or covered, when no UI pass runs: closing
    /// from the taskbar, and the title staying true.
    fn background(&mut self, ctx: &Context) {
        self.take_updates(ctx);
        self.handle_close(ctx);
        if self.draw_hidden && ctx.input(|input| input.viewport().visible()) == Some(false) {
            ctx.send_viewport_cmd(ViewportCommand::RequestPaintWhileHidden);
        }
    }

    /// Draws everything. Separate from `eframe::App` so tests can drive it without a window.
    pub fn show(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.background(&ctx);
        self.hotkeys(&ctx);
        egui::Panel::top("header").show(ui, |ui| self.header(ui));
        egui::CentralPanel::default().show(ui, |ui| self.body(ui));
    }

    #[cfg(test)]
    pub fn page(&self) -> Page {
        self.page
    }

    #[cfg(test)]
    pub fn is_loaded(&self) -> bool {
        self.data.is_some()
    }

    #[cfg(test)]
    pub fn data(&self) -> Option<&ViewData> {
        self.data.as_deref()
    }

    #[cfg(test)]
    pub fn report_text(&self) -> String {
        self.report.join("\n")
    }

    #[cfg(test)]
    pub fn session(&self) -> &Handle {
        &self.session
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.background(ctx);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.frames += 1;
        self.show(ui);
    }

    fn on_exit(&mut self) {
        log!("closing after drawing {} frames", self.frames);
    }
}

fn applied_message(reply: Option<Reply>) -> String {
    match reply {
        Some(Ok(output)) => format!("Saved: {}", output.render().join(" ")),
        Some(Err(error)) => format!("Not changed: {error}"),
        None => "The session has closed.".to_string(),
    }
}

/// Refreshes the view whenever the session reports a change. It's a separate thread so the UI thread
/// never waits on the session, and repaints are only asked for when something really changed.
fn view_loop(session: &Handle, changes: &Receiver<()>, updates: &Sender<Update>, ctx: &Context) {
    while changes.recv().is_ok() {
        while changes.try_recv().is_ok() {}
        let servers = session.query("list servers").recv();
        let status = session.query("show status").recv();
        let config = session.query("show config").recv();
        if let (
            Ok(Ok(Output::Servers(servers))),
            Ok(Ok(Output::Status(status))),
            Ok(Ok(Output::Config(config))),
        ) = (servers, status, config)
        {
            let view = rows::build_view(&servers, &status);
            let data = ViewData {
                servers,
                status,
                config,
                view,
            };
            if updates.send(Update::View(Box::new(data))).is_err() {
                return;
            }
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applied_messages_say_what_happened() {
        assert_eq!(
            applied_message(Some(Ok(Output::Lines(vec!["ping_every = 30".to_string()])))),
            "Saved: ping_every = 30"
        );
        assert_eq!(
            applied_message(Some(Err("ping_every must be 0 to 300".to_string()))),
            "Not changed: ping_every must be 0 to 300"
        );
        assert!(applied_message(None).contains("closed"));
    }
}
