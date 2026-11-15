//! Drives the real UI code in-process with a fake firewall and no network. No real window, mouse or
//! focus is involved, so nothing on the PC can disturb these.

use super::app::{App, Page};
use super::theme;
use crate::sdr::{self, Blocks};
use crate::session::{self, Firewall};
use eframe::egui::{self, Key, Theme};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RELAYS: &str = r#"{"pops":{
    "ams":{"desc":"Amsterdam (Netherlands)","relays":[{"ipv4":"1.1.1.1"}]},
    "fra":{"desc":"Frankfurt (Germany)","relays":[{"ipv4":"2.2.2.2"}]},
    "dfra":{"desc":"Datapacket Frankfurt (Germany)","relays":[{"ipv4":"2.2.2.3"}]},
    "jnb":{"desc":"Johannesburg (South Africa)","relays":[{"ipv4":"3.3.3.3"}]},
    "sgp":{"desc":"Singapore","relays":[{"ipv4":"4.4.4.4"}]},
    "iad":{"desc":"Sterling (Virginia)","relays":[{"ipv4":"5.5.5.5"}]}
}}"#;
const NO_BLOCK: &str = "No block - all servers";

#[derive(Clone, Default)]
struct FakeFirewall(Arc<Mutex<Option<String>>>);

impl Firewall for FakeFirewall {
    fn stamp(&self) -> Option<String> {
        self.0.lock().unwrap().clone()
    }
    fn apply(&self, stamp: &str, _blocks: &Blocks) -> Result<(), String> {
        *self.0.lock().unwrap() = Some(stamp.to_string());
        Ok(())
    }
    fn clear(&self) -> Result<(), String> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }
    fn watch_drops(&self, _on: bool) -> Result<(), String> {
        Ok(())
    }
}

fn harness(firewall: FakeFirewall) -> Harness<'static, App> {
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let app = App::start(ctx, |host| session::spawn_offline(firewall, host));
    app.session().inject_relays(sdr::parse(RELAYS).unwrap());
    let mut harness = Harness::builder()
        .with_size(egui::vec2(720.0, 700.0))
        .build_ui_state(|ui, app: &mut App| app.show(ui), app);
    wait(&mut harness, |app| app.is_loaded());
    harness
}

/// Steps the UI until background work finishes.
fn wait(harness: &mut Harness<'static, App>, done: impl Fn(&App) -> bool) {
    let until = Instant::now() + Duration::from_secs(10);
    while !done(harness.state()) {
        assert!(Instant::now() < until, "timed out waiting for the UI");
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.step();
    harness.step();
}

fn press(harness: &mut Harness<'static, App>, key: Key) {
    harness.key_press(key);
    harness.step();
    harness.step();
}

fn pick(app: &App) -> String {
    app.data()
        .map_or_else(String::new, |data| data.status.pick.clone())
}

/// Clicks the box and types into it.
fn search(harness: &mut Harness<'static, App>, text: &str) {
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Open the server list")
        .click();
    harness.step();
    harness.step();
    harness.step();
    harness.event(egui::Event::Text(text.to_string()));
    harness.step();
    harness.step();
}

#[test]
fn an_idle_window_stops_repainting_on_every_page() {
    let mut harness = harness(FakeFirewall::default());
    harness.run();
    for key in [Key::F1, Key::F2, Key::F3, Key::Escape] {
        harness.key_press(key);
        harness.run();
    }
    harness.event(egui::Event::Text("`".to_string()));
    wait(&mut harness, |app| app.page() == Page::Console);
    harness.run();
}

#[test]
fn the_dropdown_opens_with_sections_and_no_block_first() {
    let mut harness = harness(FakeFirewall::default());
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Open the server list")
        .click();
    harness.step();
    harness.step();
    harness.get_by_label(NO_BLOCK);
    harness.get_by_label("Africa");
    harness.get_by_label("Europe");
    harness.get_by_label("Johannesburg (South Africa) - jnb");
}

#[test]
fn typing_filters_the_list() {
    let mut harness = harness(FakeFirewall::default());
    search(&mut harness, "frank");
    harness.get_by_label("Frankfurt (Germany) - fra");
    harness.get_by_label("Europe");
    assert!(harness.query_by_label("Singapore - sgp").is_none());
    assert!(harness.query_by_label("Africa").is_none());
    assert!(harness.query_by_label(NO_BLOCK).is_none());
}

#[test]
fn a_search_can_match_a_region_or_a_code() {
    let mut harness = harness(FakeFirewall::default());
    search(&mut harness, "africa");
    harness.get_by_label("Johannesburg (South Africa) - jnb");
    assert!(harness.query_by_label("Singapore - sgp").is_none());
}

#[test]
fn a_search_with_no_match_says_so_and_enter_does_nothing() {
    let mut harness = harness(FakeFirewall::default());
    search(&mut harness, "zzzz");
    harness.get_by_label("No matching servers");
    press(&mut harness, Key::Enter);
    press(&mut harness, Key::Escape);
    harness.step();
    assert_eq!(pick(harness.state()), "");
}

#[test]
fn enter_picks_the_first_match_and_blocks_for_real() {
    let firewall = FakeFirewall::default();
    let mut harness = harness(firewall.clone());
    search(&mut harness, "sterl");
    press(&mut harness, Key::Enter);
    wait(&mut harness, |app| pick(app) == "iad");
    assert!(firewall.stamp().unwrap().starts_with("iad "));
    assert!(
        harness.query_by_label("Singapore - sgp").is_none(),
        "list closed"
    );
}

#[test]
fn escape_cancels_a_search_without_changing_the_pick() {
    let firewall = FakeFirewall::default();
    let mut harness = harness(firewall.clone());
    search(&mut harness, "sterl");
    press(&mut harness, Key::Escape);
    harness.step();
    assert!(
        harness
            .query_by_label("Sterling (Virginia) - iad")
            .is_none()
    );
    assert_eq!(pick(harness.state()), "");
    assert_eq!(firewall.stamp(), None);
}

#[test]
fn arrow_keys_skip_headers() {
    let mut harness = harness(FakeFirewall::default());
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Open the server list")
        .click();
    harness.step();
    harness.step();
    // From No block, down lands on the first real site under Africa, not on the header.
    press(&mut harness, Key::ArrowDown);
    press(&mut harness, Key::Enter);
    wait(&mut harness, |app| pick(app) == "jnb");
}

#[test]
fn clicking_a_header_picks_nothing_and_clicking_a_server_does() {
    let firewall = FakeFirewall::default();
    let mut harness = harness(firewall.clone());
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Open the server list")
        .click();
    harness.step();
    harness.step();
    harness.get_by_label("Africa").click();
    harness.step();
    harness.step();
    assert_eq!(firewall.stamp(), None);
    harness.get_by_label("Singapore - sgp").click();
    harness.step();
    wait(&mut harness, |app| pick(app) == "sgp");
    assert!(firewall.stamp().unwrap().starts_with("sgp "));
}

#[test]
fn choosing_the_current_pick_issues_nothing() {
    let firewall = FakeFirewall::default();
    let mut harness = harness(firewall.clone());
    search(&mut harness, "sgp");
    press(&mut harness, Key::Enter);
    wait(&mut harness, |app| pick(app) == "sgp");
    let stamp = firewall.stamp();
    search(&mut harness, "sgp");
    press(&mut harness, Key::Enter);
    harness.step();
    assert_eq!(firewall.stamp(), stamp);
}

#[test]
fn no_block_clears_a_pick() {
    let firewall = FakeFirewall::default();
    let mut harness = harness(firewall.clone());
    search(&mut harness, "sgp");
    press(&mut harness, Key::Enter);
    wait(&mut harness, |app| pick(app) == "sgp");
    search(&mut harness, "no block");
    press(&mut harness, Key::Enter);
    wait(&mut harness, |app| pick(app).is_empty());
    assert_eq!(firewall.stamp(), None);
}

#[test]
fn f_keys_switch_pages_and_esc_returns_to_status() {
    let mut harness = harness(FakeFirewall::default());
    assert_eq!(harness.state().page(), Page::Status);
    press(&mut harness, Key::F1);
    assert_eq!(harness.state().page(), Page::Help);
    press(&mut harness, Key::F2);
    assert_eq!(harness.state().page(), Page::Settings);
    press(&mut harness, Key::F3);
    assert_eq!(harness.state().page(), Page::Report);
    press(&mut harness, Key::Escape);
    assert_eq!(harness.state().page(), Page::Status);
}

#[test]
fn backtick_opens_the_console_and_never_types() {
    let mut harness = harness(FakeFirewall::default());
    harness.event(egui::Event::Text("`".to_string()));
    wait(&mut harness, |app| app.page() == Page::Console);
    harness.event(egui::Event::Text("`".to_string()));
    wait(&mut harness, |app| app.page() == Page::Status);
}

#[test]
fn the_console_runs_a_typed_command() {
    let firewall = FakeFirewall::default();
    let mut harness = harness(firewall.clone());
    harness.event(egui::Event::Text("`".to_string()));
    wait(&mut harness, |app| app.page() == Page::Console);
    harness.step();
    harness.step();
    harness.event(egui::Event::Text("set pick fra".to_string()));
    harness.step();
    harness.step();
    harness.key_press(Key::Enter);
    wait(&mut harness, |app| pick(app) == "fra");
    assert!(firewall.stamp().unwrap().starts_with("fra "));
}

#[test]
fn esc_in_the_console_clears_a_typed_line_first() {
    let mut harness = harness(FakeFirewall::default());
    harness.event(egui::Event::Text("`".to_string()));
    wait(&mut harness, |app| app.page() == Page::Console);
    harness.step();
    harness.step();
    harness.event(egui::Event::Text("set".to_string()));
    harness.step();
    harness.step();
    press(&mut harness, Key::Escape);
    assert_eq!(harness.state().page(), Page::Console);
    press(&mut harness, Key::Escape);
    wait(&mut harness, |app| app.page() == Page::Status);
}

fn open_console(harness: &mut Harness<'static, App>) {
    harness.event(egui::Event::Text("`".to_string()));
    wait(harness, |app| app.page() == Page::Console);
    harness.step();
    harness.step();
}

fn prompt_text(harness: &Harness<'static, App>) -> String {
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "dl-conn-rs>")
        .value()
        .unwrap_or_default()
}

#[test]
fn tab_and_history_work_in_the_console() {
    let mut harness = harness(FakeFirewall::default());
    open_console(&mut harness);
    harness.event(egui::Event::Text("list se".to_string()));
    harness.step();
    harness.step();
    press(&mut harness, Key::Tab);
    harness.step();
    assert_eq!(prompt_text(&harness), "list servers ");
    press(&mut harness, Key::Enter);
    harness.step();
    assert_eq!(prompt_text(&harness), "");
    press(&mut harness, Key::ArrowUp);
    harness.step();
    assert_eq!(prompt_text(&harness), "list servers");
    assert_eq!(
        harness.state().page(),
        Page::Console,
        "keys stay in the prompt"
    );
}

#[test]
fn a_question_mark_never_types() {
    let mut harness = harness(FakeFirewall::default());
    open_console(&mut harness);
    harness.event(egui::Event::Text("set pick".to_string()));
    harness.step();
    harness.step();
    harness.event(egui::Event::Text("?".to_string()));
    harness.step();
    harness.step();
    harness.step();
    // What it explains is tested in command.rs. The log is shared between tests, so it can't be read here.
    assert_eq!(prompt_text(&harness), "set pick");
}

#[test]
fn text_set_from_outside_opens_the_search() {
    let firewall = FakeFirewall::default();
    let mut harness = harness(firewall.clone());
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, "Server")
        .set_text_value_accesskit("sterl");
    harness.step();
    harness.step();
    harness.step();
    harness.get_by_label("Sterling (Virginia) - iad");
    assert!(harness.query_by_label("Singapore - sgp").is_none());
    press(&mut harness, Key::Enter);
    wait(&mut harness, |app| pick(app) == "iad");
    assert!(firewall.stamp().unwrap().starts_with("iad "));
}

#[test]
fn leaving_the_console_by_f_key_stops_its_report() {
    let mut harness = harness(FakeFirewall::default());
    harness.event(egui::Event::Text("`".to_string()));
    wait(&mut harness, |app| app.page() == Page::Console);
    press(&mut harness, Key::F1);
    assert_eq!(harness.state().page(), Page::Help);
    harness.event(egui::Event::Text("`".to_string()));
    wait(&mut harness, |app| app.page() == Page::Console);
}

#[test]
fn help_shows_the_guide() {
    let mut harness = harness(FakeFirewall::default());
    press(&mut harness, Key::F1);
    let until = Instant::now() + Duration::from_secs(5);
    while harness
        .query_by_label_contains("STEP 3: TEST IN A CUSTOM GAME")
        .is_none()
    {
        assert!(Instant::now() < until);
        harness.step();
    }
}

#[test]
fn the_report_explains_a_missing_log() {
    let folder = data_folder("report");
    let mut harness = harness(FakeFirewall::default());
    let missing = folder.join("nothing.log").display().to_string();
    let reply = harness
        .state()
        .session()
        .query(&format!("set config log_path {missing}"))
        .recv()
        .unwrap();
    assert!(reply.is_ok());
    press(&mut harness, Key::F3);
    wait(&mut harness, |app| app.report_text().contains("No log at"));
    assert!(harness.state().report_text().contains("nothing.log"));
}

/// Replaces the text in a labelled box through the real form, as typing would.
fn type_into(harness: &mut Harness<'static, App>, label: &str, text: &str) {
    harness
        .get_by_role_and_label(egui::accesskit::Role::TextInput, label)
        .focus();
    harness.step();
    harness.step();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
    harness.event(egui::Event::Text(text.to_string()));
    harness.step();
    harness.step();
    harness.key_press(Key::Enter);
    harness.step();
    harness.step();
}

fn setting(app: &App, key: &str) -> String {
    app.data()
        .and_then(|data| data.config.iter().find(|row| row.key == key))
        .map_or_else(String::new, |row| row.value.clone())
}

#[test]
fn settings_save_good_values_and_explain_bad_ones() {
    let mut harness = harness(FakeFirewall::default());
    press(&mut harness, Key::F2);
    harness.step();
    let label = "Direct ping interval (s)";
    type_into(&mut harness, label, "30");
    wait(&mut harness, |app| setting(app, "ping_every") == "30");
    let until = Instant::now() + Duration::from_secs(5);
    while harness
        .query_by_label_contains("Saved: ping_every = 30")
        .is_none()
    {
        assert!(Instant::now() < until);
        harness.step();
    }

    type_into(&mut harness, label, "999");
    let until = Instant::now() + Duration::from_secs(5);
    while harness
        .query_by_label_contains("Not changed: ping_every must be 0 to 300")
        .is_none()
    {
        assert!(Instant::now() < until);
        harness.step();
    }
    assert_eq!(setting(harness.state(), "ping_every"), "30");
    harness.step();
    let field = harness.get_by_role_and_label(egui::accesskit::Role::TextInput, label);
    assert_eq!(field.value().as_deref(), Some("30"), "the box goes back");
}

#[test]
fn the_settings_form_shows_every_setting() {
    let mut harness = harness(FakeFirewall::default());
    press(&mut harness, Key::F2);
    harness.step();
    for title in [
        "Default backup site",
        "Direct ping interval (s)",
        "Relay refresh (minutes)",
        "Hold filter updates in game",
        "Console report interval (s)",
        "Report when console opens",
        "Remove block on exit",
        "Game log path",
    ] {
        assert!(
            harness.query_all_by_label(title).next().is_some(),
            "{title}"
        );
    }
    assert!(harness.query_all_by_label("Theme").next().is_some());
}

#[test]
fn the_theme_setting_changes_the_window() {
    let mut harness = harness(FakeFirewall::default());
    let ctx_dark = |harness: &Harness<'static, App>| harness.ctx.theme() == Theme::Dark;
    harness.state().session().run("set config theme light");
    wait(&mut harness, |app| {
        app.data().is_some_and(|data| {
            data.config
                .iter()
                .any(|row| row.key == "theme" && row.value == "light")
        })
    });
    harness.step();
    assert!(!ctx_dark(&harness));
    harness.state().session().run("set config theme dark");
    wait(&mut harness, |app| {
        app.data().is_some_and(|data| {
            data.config
                .iter()
                .any(|row| row.key == "theme" && row.value == "dark")
        })
    });
    harness.step();
    assert!(ctx_dark(&harness));
}

#[test]
fn it_works_with_no_exitlag_no_steam_and_no_log() {
    let mut harness = harness(FakeFirewall::default());
    assert!(
        harness.query_by_label("Direct").is_none(),
        "nothing to ping"
    );
    harness.get_by_label("Game");
    harness.get_by_label("Connection Hold");
    assert_eq!(harness.query_all_by_label("No log").count(), 2);
    assert!(harness.query_by_label_contains("ExitLag").is_none());
    search(&mut harness, "sgp");
    press(&mut harness, Key::Enter);
    wait(&mut harness, |app| pick(app) == "sgp");
    press(&mut harness, Key::F1);
    let until = Instant::now() + Duration::from_secs(5);
    while harness.query_by_label_contains("STEP 1").is_none() {
        assert!(Instant::now() < until);
        harness.step();
    }
}

fn data_folder(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("dl-conn-rs-ui-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// Saves a picture of each page in both modes, for looking at by eye. It needs a GPU or the Windows
/// software renderer, so it only runs when asked for.
#[test]
#[ignore = "writes screenshots; set DL_CONN_RS_SHOTS to a folder and run by hand"]
fn screenshots() {
    let folder = PathBuf::from(std::env::var_os("DL_CONN_RS_SHOTS").expect("DL_CONN_RS_SHOTS"));
    std::fs::create_dir_all(&folder).unwrap();
    for theme_name in ["light", "dark"] {
        let firewall = FakeFirewall::default();
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let app = App::start(ctx, |host| session::spawn_offline(firewall, host));
        app.session().inject_relays(sdr::parse(RELAYS).unwrap());
        app.session()
            .query(&format!("set config theme {theme_name}"))
            .recv()
            .unwrap()
            .unwrap();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(720.0, 700.0))
            .wgpu()
            .build_ui_state(|ui, app: &mut App| app.show(ui), app);
        wait(&mut harness, |app| {
            app.data().is_some_and(|data| {
                data.config
                    .iter()
                    .any(|row| row.key == "theme" && row.value == theme_name)
            })
        });
        let shot = |harness: &mut Harness<'static, App>, name: &str| {
            for _ in 0..4 {
                harness.step();
                std::thread::sleep(Duration::from_millis(60));
            }
            let image = harness.render().unwrap();
            image
                .save(folder.join(format!("{theme_name}-{name}.png")))
                .unwrap();
        };
        shot(&mut harness, "status");
        search(&mut harness, "");
        shot(&mut harness, "list");
        harness.event(egui::Event::Text("frank".to_string()));
        harness.step();
        shot(&mut harness, "search");
        press(&mut harness, Key::Enter);
        wait(&mut harness, |app| !pick(app).is_empty());
        shot(&mut harness, "picked");
        let site = pick(harness.state());
        let mut state = game_state(
            Some((crate::analysis::RouteState::Measured, &site)),
            &format!("{site}=121"),
        );
        state.exitlag = true;
        state.direct = crate::monitor::Direct::Ms(351);
        harness.state().session().inject_monitor(state);
        wait(&mut harness, |app| hold(app).as_deref() == Some("Ready"));
        shot(&mut harness, "chips");
        harness.get_by_label("Ready").hover();
        for _ in 0..10 {
            harness.step();
            std::thread::sleep(Duration::from_millis(60));
        }
        shot(&mut harness, "tooltip");
        press(&mut harness, Key::F1);
        std::thread::sleep(Duration::from_millis(100));
        shot(&mut harness, "help");
        press(&mut harness, Key::F2);
        shot(&mut harness, "settings");
        press(&mut harness, Key::F3);
        std::thread::sleep(Duration::from_millis(200));
        shot(&mut harness, "report");
        press(&mut harness, Key::Escape);
        harness.event(egui::Event::Text("`".to_string()));
        wait(&mut harness, |app| app.page() == Page::Console);
        harness.event(egui::Event::Text("help".to_string()));
        harness.step();
        harness.key_press(Key::Enter);
        std::thread::sleep(Duration::from_millis(200));
        shot(&mut harness, "console");
    }
}

/// What the monitor reports while the game runs, with its log showing `route` and `table`.
fn game_state(
    route: Option<(crate::analysis::RouteState, &str)>,
    table: &str,
) -> crate::monitor::Monitor {
    let table = table
        .split(',')
        .filter_map(|entry| {
            let (site, ms) = entry.split_once('=')?;
            Some(crate::analysis::PingEntry {
                site: site.to_string(),
                direct: ms.parse().ok(),
                via: None,
                penalty: 0,
            })
        })
        .collect();
    crate::monitor::Monitor {
        game: true,
        game_log: crate::monitor::GameLog::Writing,
        log_found: true,
        route: route.map(|(state, site)| crate::analysis::Route {
            site: site.to_string(),
            ms: 120,
            at: "18:46".to_string(),
            state,
        }),
        table,
        ..crate::monitor::Monitor::default()
    }
}

/// The Connection Hold chip's value.
fn hold(app: &App) -> Option<String> {
    let data = app.data()?;
    let chip = data
        .view
        .chips
        .iter()
        .find(|chip| chip.name == crate::monitor::HOLD)?;
    Some(chip.value.clone())
}

#[test]
fn the_header_says_when_the_game_measured_before_the_pick() {
    let mut harness = harness(FakeFirewall::default());
    search(&mut harness, "jnb");
    press(&mut harness, Key::Enter);
    wait(&mut harness, |app| pick(app) == "jnb");
    harness
        .state()
        .session()
        .inject_monitor(game_state(None, "sgp=119,jnb="));
    wait(&mut harness, |app| {
        hold(app).as_deref() == Some("Restart game")
    });
    harness.get_by_label("Restart game").hover();
    for _ in 0..20 {
        harness.step();
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        harness
            .query_by_label_contains("only measures when it starts")
            .is_some(),
        "the chip explains itself on hover"
    );

    harness
        .state()
        .session()
        .inject_monitor(game_state(None, "jnb=4,sgp="));
    wait(&mut harness, |app| hold(app).as_deref() == Some("Ready"));
}

#[test]
fn a_pick_mid_match_waits_and_the_header_says_so() {
    let firewall = FakeFirewall::default();
    let mut harness = harness(firewall.clone());
    search(&mut harness, "sgp");
    press(&mut harness, Key::Enter);
    wait(&mut harness, |app| pick(app) == "sgp");
    let session = harness.state().session().clone();
    session.inject_monitor(game_state(
        Some((crate::analysis::RouteState::InMatch, "sgp")),
        "sgp=119",
    ));
    wait(&mut harness, |app| hold(app).as_deref() == Some("In match"));
    search(&mut harness, "jnb");
    press(&mut harness, Key::Enter);
    wait(&mut harness, |app| {
        hold(app).as_deref() == Some("jnb after match")
    });
    assert_eq!(pick(harness.state()), "sgp", "the match isn't cut");
    assert!(firewall.stamp().unwrap().starts_with("sgp "));

    session.inject_monitor(game_state(
        Some((crate::analysis::RouteState::LastMatch, "sgp")),
        "sgp=119",
    ));
    wait(&mut harness, |app| pick(app) == "jnb");
    assert!(firewall.stamp().unwrap().starts_with("jnb "));
}
