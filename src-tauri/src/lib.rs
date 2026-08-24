mod blocks;
mod settings;
mod usage;

use blocks::Snapshot;
use std::sync::Mutex;
use tauri::{Manager, State};

#[derive(Default)]
struct App {
    scanner: Mutex<usage::Scanner>,
    config_dir: Mutex<std::path::PathBuf>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn read_events(state: &State<App>) -> Result<Vec<usage::Event>, String> {
    let root = usage::projects_dir()
        .ok_or("No Claude Code session data found at ~/.claude/projects.")?;
    let mut scanner = state.scanner.lock().map_err(|e| e.to_string())?;
    Ok(scanner.scan(&root))
}

fn snapshot_from(state: &State<App>, events: &[usage::Event]) -> Result<Snapshot, String> {
    let dir = state.config_dir.lock().map_err(|e| e.to_string())?.clone();
    let cfg = settings::load(&dir);
    let (limit, weekly, cal_s, cal_w) = settings::effective(&cfg, events);
    Ok(blocks::build(events, now_ms(), limit, weekly, (cal_s, cal_w)))
}

#[tauri::command]
fn get_snapshot(state: State<App>) -> Result<Snapshot, String> {
    let events = read_events(&state)?;
    snapshot_from(&state, &events)
}

/// Solves for the real limit from the percentage Claude Code reports.
/// `session_pct` is what `/usage` shows for the 5-hour block right now;
/// `weekly_pct` is the weekly figure. Either may be omitted.
#[tauri::command]
fn calibrate(
    state: State<App>,
    session_pct: Option<f64>,
    weekly_pct: Option<f64>,
) -> Result<Snapshot, String> {
    let events = read_events(&state)?;
    let dir = state.config_dir.lock().map_err(|e| e.to_string())?.clone();
    let mut cfg = settings::load(&dir);
    let now = now_ms();

    if let Some(pct) = session_pct {
        if !(0.5..=100.0).contains(&pct) {
            return Err("Enter the session percentage /usage shows, between 0.5 and 100.".into());
        }
        let start = blocks::active_block_start(&events, now)
            .ok_or("No block is open right now, so there is nothing to calibrate against. Send a message first.")?;
        let counted: f64 = events
            .iter()
            .filter(|e| e.ts_ms >= start && e.ts_ms < start + blocks::BLOCK_MS)
            .map(|e| e.weighted)
            .sum();
        if counted <= 0.0 {
            return Err("No usage counted in the current block yet.".into());
        }
        cfg.limit_tokens = Some(counted / (pct / 100.0));
    }

    if let Some(pct) = weekly_pct {
        if !(0.5..=100.0).contains(&pct) {
            return Err("Enter the weekly percentage /usage shows, between 0.5 and 100.".into());
        }
        let week_ms = 7 * 24 * 60 * 60 * 1000;
        let counted: f64 = events
            .iter()
            .filter(|e| e.ts_ms > now - week_ms)
            .map(|e| e.weighted)
            .sum();
        if counted <= 0.0 {
            return Err("No usage counted in the last seven days yet.".into());
        }
        cfg.weekly_limit_tokens = Some(counted / (pct / 100.0));
    }

    cfg.calibrated_at_ms = Some(now);
    settings::save(&dir, &cfg).map_err(|e| e.to_string())?;
    snapshot_from(&state, &events)
}

#[tauri::command]
fn reset_calibration(state: State<App>) -> Result<Snapshot, String> {
    let dir = state.config_dir.lock().map_err(|e| e.to_string())?.clone();
    settings::save(&dir, &settings::Settings::default()).map_err(|e| e.to_string())?;
    let events = read_events(&state)?;
    snapshot_from(&state, &events)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(App::default())
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            *app.state::<App>().config_dir.lock().unwrap() = dir;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            calibrate,
            reset_calibration
        ])
        .run(tauri::generate_context!())
        .expect("error while running claude-usage-monitor");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises the real pipeline against this machine's session logs.
    #[test]
    fn snapshot_from_local_sessions() {
        let root = usage::projects_dir().expect("projects dir");
        let mut sc = usage::Scanner::default();
        let ev = sc.scan(&root);
        assert!(!ev.is_empty(), "no events parsed");

        // Second scan must be incremental and identical.
        let ev2 = sc.scan(&root);
        assert_eq!(ev.len(), ev2.len(), "incremental rescan changed event count");

        let now = now_ms();
        let (limit, weekly, _, _) = settings::effective(&settings::Settings::default(), &ev);
        let snap = blocks::build(&ev, now, limit, weekly, (false, false));

        eprintln!("events={} limit={:.0} used={:.1}% burn={:.1}%/hr proj={:.1}% msgs={} models={:?}",
            ev.len(), limit, snap.used_pct, snap.burn_pct_per_hour,
            snap.projected_end_pct, snap.messages,
            snap.models.iter().map(|m| (&m.label, (m.pct*10.0).round()/10.0)).collect::<Vec<_>>());
        eprintln!("block={:?} filled={}/{} resets_in={}s weekly={:.1}%",
            snap.block_start_ms, snap.filled_buckets, blocks::BUCKETS,
            snap.resets_in_seconds, snap.weekly.pct);

        // Series must be cumulative and consistent with the totals.
        for s in &snap.series {
            assert_eq!(s.values.len(), snap.filled_buckets, "series length mismatch");
            for w in s.values.windows(2) {
                assert!(w[1] >= w[0] - 1e-9, "series must not decrease inside a block");
            }
        }
        let tail: f64 = snap.series.iter().filter_map(|s| s.values.last()).sum();
        assert!((tail - snap.used_pct).abs() < 0.05,
            "series tail {tail} disagrees with used_pct {}", snap.used_pct);
        assert!(snap.used_pct <= 100.0 / 1.05 + 0.01, "uncalibrated usage should sit under the provisional ceiling");
    }

    /// The calibration contract: entering the percentage /usage reports
    /// must make our own count land on exactly that percentage.
    #[test]
    fn calibration_round_trips() {
        let root = usage::projects_dir().expect("projects dir");
        let ev = usage::Scanner::default().scan(&root);
        let now = now_ms();
        let Some(start) = blocks::active_block_start(&ev, now) else { return };
        let counted: f64 = ev
            .iter()
            .filter(|e| e.ts_ms >= start && e.ts_ms < start + blocks::BLOCK_MS)
            .map(|e| e.weighted)
            .sum();

        for observed in [12.5_f64, 50.0, 99.0] {
            let solved = counted / (observed / 100.0);
            let snap = blocks::build(&ev, now, solved, 1.0, (true, false));
            assert!((snap.used_pct - observed).abs() < 0.01,
                "calibrating to {observed}% produced {}%", snap.used_pct);
        }
    }

    #[test]
    fn settings_round_trip_and_fall_back() {
        let root = usage::projects_dir().expect("projects dir");
        let ev = usage::Scanner::default().scan(&root);

        // Uncalibrated: provisional ceiling sits above the busiest block.
        let (limit, _, cal, _) = settings::effective(&settings::Settings::default(), &ev);
        assert!(!cal);
        assert!(limit > settings::busiest_block(&ev), "provisional limit needs headroom");

        // Calibrated values win over the provisional ones.
        let cfg = settings::Settings {
            limit_tokens: Some(42_000_000.0),
            weekly_limit_tokens: Some(500_000_000.0),
            calibrated_at_ms: Some(0),
        };
        let (limit, weekly, cal_s, cal_w) = settings::effective(&cfg, &ev);
        assert_eq!((limit, weekly, cal_s, cal_w), (42_000_000.0, 500_000_000.0, true, true));

        // A zero or negative stored limit must not divide by zero.
        let bad = settings::Settings { limit_tokens: Some(0.0), ..Default::default() };
        let (limit, _, cal, _) = settings::effective(&bad, &ev);
        assert!(limit > 0.0 && !cal, "a zero limit must fall back, not stick");

        let dir = std::env::temp_dir().join("cum-test-settings");
        settings::save(&dir, &cfg).expect("save");
        assert_eq!(settings::load(&dir).limit_tokens, Some(42_000_000.0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn labels_are_readable() {
        assert_eq!(blocks::label_for("claude-opus-5"), "Opus 5");
        assert_eq!(blocks::label_for("claude-haiku-4-5-20251001"), "Haiku 4.5");
        assert_eq!(blocks::label_for("claude-sonnet-5"), "Sonnet 5");
    }
}
