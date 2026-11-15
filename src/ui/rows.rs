//! What the main window shows, worked out without touching Windows.

use crate::monitor::Chip;
use crate::regions;
use crate::session::{Server, Status};

pub const NO_BLOCK_LABEL: &str = "No block - all servers";

/// One line of the dropdown.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The server code, empty for "No block". `None` makes this a section header.
    pub code: Option<String>,
    pub label: String,
}

impl Row {
    fn header(region: &str) -> Self {
        Self {
            code: None,
            label: region.to_string(),
        }
    }

    pub fn is_header(&self) -> bool {
        self.code.is_none()
    }
}

/// What the main window should show.
#[derive(Debug, PartialEq)]
pub struct View {
    pub rows: Vec<Row>,
    pub selected: Option<usize>,
    pub title: String,
    /// The counters and the hold, each a short value with an explanation on hover.
    pub chips: Vec<Chip>,
}

/// Whether every word typed appears in the label or the region, so "asia sing" finds Singapore.
fn matches(query: &str, label: &str, region: &str) -> bool {
    let (label, region) = (label.to_lowercase(), region.to_lowercase());
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| label.contains(word) || region.contains(word))
}

/// The dropdown's rows, grouped by region and narrowed to what's been typed. A section with nothing
/// to show is left out. A pick the relay list no longer has stays visible, so the dropdown never
/// misstates what's active.
pub fn build_rows(servers: &[Server], status: &Status, query: &str) -> Vec<Row> {
    let mut sites: Vec<(&'static str, String, String)> = servers
        .iter()
        .map(|server| {
            let label = format!("{} - {}", server.desc, server.code);
            (
                regions::of(&server.code, &server.desc),
                label,
                server.code.clone(),
            )
        })
        .collect();
    if !status.pick.is_empty() && status.pick_desc.is_none() {
        sites.push((
            regions::of(&status.pick, ""),
            status.pick.clone(),
            status.pick.clone(),
        ));
    }
    sites.retain(|(region, label, _)| matches(query, label, region));
    sites.sort_by(|a, b| a.1.cmp(&b.1));

    let mut rows = Vec::new();
    if matches(query, NO_BLOCK_LABEL, "") {
        rows.push(Row {
            code: Some(String::new()),
            label: NO_BLOCK_LABEL.to_string(),
        });
    }
    for region in regions::ORDER {
        let mut members = sites.iter().filter(|site| site.0 == region).peekable();
        if members.peek().is_some() {
            rows.push(Row::header(region));
            rows.extend(members.map(|(_, label, code)| Row {
                code: Some(code.clone()),
                label: label.clone(),
            }));
        }
    }
    rows
}

/// Where a server is in the rows.
pub fn position(rows: &[Row], code: &str) -> Option<usize> {
    rows.iter()
        .position(|row| row.code.as_deref() == Some(code))
}

/// The first row that can be picked. Typing highlights it, and Enter picks it.
pub fn first_match(rows: &[Row], query: &str) -> Option<usize> {
    let query = query.trim();
    let is_real = |row: &Row| row.code.as_deref().is_some_and(|code| !code.is_empty());
    if query.is_empty() {
        return rows.iter().position(|row| !row.is_header());
    }
    rows.iter()
        .position(is_real)
        .or_else(|| rows.iter().position(|row| !row.is_header()))
}

/// The next row an arrow key lands on, skipping headers and stopping at the ends.
pub fn step(rows: &[Row], from: Option<usize>, down: bool) -> Option<usize> {
    let Some(from) = from else {
        return rows.iter().position(|row| !row.is_header());
    };
    let next = if down {
        (from + 1..rows.len()).find(|at| !rows[*at].is_header())
    } else {
        (0..from).rev().find(|at| !rows[*at].is_header())
    };
    next.or(Some(from))
}

pub fn build_view(servers: &[Server], status: &Status) -> View {
    let rows = build_rows(servers, status, "");
    View {
        selected: position(&rows, &status.pick),
        rows,
        title: title(status),
        chips: status.chips(),
    }
}

pub fn title(status: &Status) -> String {
    if let Some(error) = &status.error {
        return format!("dl-conn-rs - {error}");
    }
    let offline = if status.offline { " (offline)" } else { "" };
    if status.pick.is_empty() {
        if !status.loaded && !status.offline {
            return "dl-conn-rs - loading".to_string();
        }
        return format!("dl-conn-rs - no block{offline}");
    }
    let backup = status
        .backup
        .as_ref()
        .map_or(String::new(), |backup| format!(" + {backup}"));
    match &status.pick_desc {
        Some(desc) => format!(
            "dl-conn-rs - {desc}{backup} - {} blocked{offline}",
            status.blocked
        ),
        None => format!("dl-conn-rs - {} active{offline}", status.pick),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::GameView;
    use crate::monitor::{Direct, Monitor};

    fn server(code: &str, desc: &str) -> Server {
        Server {
            code: code.to_string(),
            desc: desc.to_string(),
            relays: 2,
        }
    }

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
            pending: None,
            view: GameView::Unknown,
        }
    }

    fn servers() -> Vec<Server> {
        vec![
            server("pvgm", "Alibaba Cloud Shanghai (China)"),
            server("jnb", "Johannesburg (South Africa)"),
            server("ams", "Amsterdam (Netherlands)"),
            server("fra", "Frankfurt (Germany)"),
            server("sgp", "Singapore"),
            server("newx", "Somewhere (Atlantis)"),
        ]
    }

    fn labels(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|row| row.label.as_str()).collect()
    }

    #[test]
    fn rows_are_grouped_by_region_with_no_block_first_and_china_near_the_end() {
        let rows = build_rows(&servers(), &status(), "");
        assert_eq!(
            labels(&rows),
            [
                NO_BLOCK_LABEL,
                "Africa",
                "Johannesburg (South Africa) - jnb",
                "Asia",
                "Singapore - sgp",
                "Europe",
                "Amsterdam (Netherlands) - ams",
                "Frankfurt (Germany) - fra",
                "China",
                "Alibaba Cloud Shanghai (China) - pvgm",
                "Other",
                "Somewhere (Atlantis) - newx",
            ]
        );
        assert_eq!(rows[0].code.as_deref(), Some(""));
        assert!(rows[1].is_header());
        assert_eq!(rows[2].code.as_deref(), Some("jnb"));
    }

    #[test]
    fn the_test_scripts_can_still_find_a_server_by_its_name() {
        let rows = build_rows(&servers(), &status(), "");
        let by_prefix = rows
            .iter()
            .find(|row| row.label.starts_with("Frankfurt (Germany)"));
        assert_eq!(by_prefix.unwrap().code.as_deref(), Some("fra"));
    }

    #[test]
    fn typing_filters_by_name_code_or_region_and_hides_empty_sections() {
        let codes = |query| {
            build_rows(&servers(), &status(), query)
                .into_iter()
                .filter_map(|row| row.code)
                .collect::<Vec<_>>()
        };
        assert_eq!(codes("frank"), ["fra"]);
        assert_eq!(codes("JNB"), ["jnb"]);
        assert_eq!(codes("europe"), ["ams", "fra"]);
        assert_eq!(codes("china shanghai"), ["pvgm"]);
        assert_eq!(codes("no block"), [""]);
        assert!(codes("nowhere at all").is_empty());
        let rows = build_rows(&servers(), &status(), "frank");
        assert_eq!(labels(&rows), ["Europe", "Frankfurt (Germany) - fra"]);
    }

    #[test]
    fn a_blank_search_shows_everything() {
        let all = build_rows(&servers(), &status(), "");
        assert_eq!(build_rows(&servers(), &status(), "   "), all);
    }

    #[test]
    fn typing_highlights_a_real_server_ahead_of_no_block() {
        let rows = build_rows(&servers(), &status(), "o");
        let first = first_match(&rows, "o").unwrap();
        assert!(!rows[first].is_header());
        assert_ne!(rows[first].code.as_deref(), Some(""));
        let only = build_rows(&servers(), &status(), "no block");
        assert_eq!(first_match(&only, "no block"), Some(0));
        assert_eq!(first_match(&[], "x"), None);
    }

    #[test]
    fn arrow_keys_skip_headers_and_stop_at_the_ends() {
        let rows = build_rows(&servers(), &status(), "");
        assert_eq!(step(&rows, Some(0), true), Some(2), "over Africa");
        assert_eq!(step(&rows, Some(2), true), Some(4), "over Asia");
        assert_eq!(step(&rows, Some(4), false), Some(2), "back over Asia");
        assert_eq!(step(&rows, Some(0), false), Some(0), "the top");
        assert_eq!(
            step(&rows, Some(rows.len() - 1), true),
            Some(rows.len() - 1)
        );
        assert_eq!(step(&rows, None, true), Some(0));
        assert_eq!(step(&[], None, true), None);
    }

    #[test]
    fn the_view_selects_the_pick_and_the_title_reports_it() {
        let view = build_view(
            &servers(),
            &Status {
                pick: "jnb".to_string(),
                pick_desc: Some("Johannesburg (South Africa)".to_string()),
                blocked: 216,
                ..status()
            },
        );
        assert_eq!(
            view.rows[view.selected.unwrap()].code.as_deref(),
            Some("jnb")
        );
        assert_eq!(
            view.title,
            "dl-conn-rs - Johannesburg (South Africa) - 216 blocked"
        );
    }

    #[test]
    fn nothing_picked_selects_no_block() {
        let view = build_view(&servers(), &status());
        assert_eq!(view.selected, Some(0));
    }

    #[test]
    fn the_title_shows_a_backup() {
        let view = build_view(
            &servers(),
            &Status {
                pick: "ams".to_string(),
                pick_desc: Some("Amsterdam (Netherlands)".to_string()),
                backup: Some("jnb".to_string()),
                blocked: 200,
                ..status()
            },
        );
        assert_eq!(
            view.rows[view.selected.unwrap()].code.as_deref(),
            Some("ams")
        );
        assert_eq!(
            view.title,
            "dl-conn-rs - Amsterdam (Netherlands) + jnb - 200 blocked"
        );
    }

    #[test]
    fn a_pick_missing_from_the_list_stays_visible() {
        let view = build_view(
            &servers(),
            &Status {
                pick: "gone".to_string(),
                ..status()
            },
        );
        assert_eq!(
            view.rows[view.selected.unwrap()].code.as_deref(),
            Some("gone")
        );
        assert_eq!(view.title, "dl-conn-rs - gone active");
    }

    #[test]
    fn the_view_carries_both_counters() {
        let view = build_view(
            &servers(),
            &Status {
                pick: "jnb".to_string(),
                pick_desc: Some("Johannesburg (South Africa)".to_string()),
                monitor: Monitor {
                    direct: Direct::Ms(180),
                    ..Monitor::default()
                },
                ..status()
            },
        );
        let chips: Vec<(&str, &str)> = view
            .chips
            .iter()
            .map(|chip| (chip.name, chip.value.as_str()))
            .collect();
        assert_eq!(
            chips,
            [
                ("Direct", "180 ms"),
                ("Game", "No log"),
                ("Connection Hold", "No log")
            ]
        );
    }

    #[test]
    fn titles_cover_every_state() {
        assert_eq!(title(&status()), "dl-conn-rs - no block");
        assert_eq!(
            title(&Status {
                loaded: false,
                ..status()
            }),
            "dl-conn-rs - loading"
        );
        assert_eq!(
            title(&Status {
                offline: true,
                ..status()
            }),
            "dl-conn-rs - no block (offline)"
        );
        assert_eq!(
            title(&Status {
                error: Some("denied".to_string()),
                ..status()
            }),
            "dl-conn-rs - denied"
        );
    }
}
