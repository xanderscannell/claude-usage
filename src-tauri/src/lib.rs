mod blocks;
mod oauth;
mod settings;
mod usage;

use blocks::Snapshot;
use std::sync::Mutex;
use tauri::{Manager, State};

/// The panel polls every few seconds, but the usage endpoint is
/// undocumented and not ours to hammer, so live readings are cached.
const LIVE_TTL_MS: i64 = 60_000;
/// How long to sit out after a failure before trying the network again.
const LIVE_RETRY_MS: i64 = 120_000;

#[derive(Default)]
struct Live {
    fetched_at_ms: i64,
    usage: Option<oauth::Usage>,
    error: Option<String>,
}

#[derive(Default)]
struct App {
    scanner: Mutex<usage::Scanner>,
    config_dir: Mutex<std::path::PathBuf>,
    live: Mutex<Live>,
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

fn config_dir(state: &State<App>) -> Result<std::path::PathBuf, String> {
    Ok(state.config_dir.lock().map_err(|e| e.to_string())?.clone())
}

/// Returns the cached live reading, refreshing it when stale. Every
/// failure is soft: the panel falls back to local logs and says why.
async fn live_usage(state: &State<'_, App>) -> (Option<oauth::Usage>, Option<String>) {
    let now = now_ms();
    {
        let cache = match state.live.lock() {
            Ok(c) => c,
            Err(_) => return (None, Some("Live usage unavailable.".into())),
        };
        let age = now - cache.fetched_at_ms;
        let ttl = if cache.usage.is_some() { LIVE_TTL_MS } else { LIVE_RETRY_MS };
        if cache.fetched_at_ms > 0 && age < ttl {
            return (cache.usage.clone(), cache.error.clone());
        }
    }

    let fetched = tauri::async_runtime::spawn_blocking(oauth::fetch).await;
    let (usage, error) = match fetched {
        Ok(Ok(u)) => (Some(u), None),
        Ok(Err(e)) => (None, Some(e.to_string())),
        Err(e) => (None, Some(format!("Live usage task failed: {e}"))),
    };

    if let Ok(mut cache) = state.live.lock() {
        cache.fetched_at_ms = now;
        cache.usage = usage.clone();
        cache.error = error.clone();
    }
    (usage, error)
}

fn assemble(
    state: &State<App>,
    events: &[usage::Event],
    live: Option<&oauth::Usage>,
    live_error: Option<String>,
) -> Result<Snapshot, String> {
    let dir = config_dir(state)?;
    let cfg = settings::load(&dir);
    let (limit, weekly_limit, cal_s, cal_w) = settings::effective(&cfg, events);

    let mut snap = blocks::build(blocks::Inputs {
        events,
        now_ms: now_ms(),
        limit,
        weekly_limit,
        session_calibrated: cal_s,
        weekly_calibrated: cal_w,
        live,
    });

    // Keep the solved limit so the panel stays accurate offline.
    settings::remember_derived(&dir, snap.derived_limit, snap.derived_weekly_limit);
    snap.live_error = live_error;
    Ok(snap)
}

#[tauri::command]
async fn get_snapshot(state: State<'_, App>) -> Result<Snapshot, String> {
    let events = read_events(&state)?;
    let (live, live_error) = live_usage(&state).await;
    assemble(&state, &events, live.as_ref(), live_error)
}

/// Solves for the limit from the percentage `/usage` reports, for when
/// the live endpoint cannot be reached. `session_pct` is the 5-hour
/// figure and `weekly_pct` the weekly one; either may be omitted.
#[tauri::command]
async fn calibrate(
    state: State<'_, App>,
    session_pct: Option<f64>,
    weekly_pct: Option<f64>,
) -> Result<Snapshot, String> {
    let events = read_events(&state)?;
    let dir = config_dir(&state)?;
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

    let (live, live_error) = live_usage(&state).await;
    assemble(&state, &events, live.as_ref(), live_error)
}

#[tauri::command]
async fn reset_calibration(state: State<'_, App>) -> Result<Snapshot, String> {
    let dir = config_dir(&state)?;
    settings::save(&dir, &settings::Settings::default()).map_err(|e| e.to_string())?;
    let events = read_events(&state)?;
    let (live, live_error) = live_usage(&state).await;
    assemble(&state, &events, live.as_ref(), live_error)
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
    use blocks::{Inputs, Source};

    fn local_events() -> Vec<usage::Event> {
        let root = usage::projects_dir().expect("projects dir");
        usage::Scanner::default().scan(&root)
    }

    fn inputs<'a>(events: &'a [usage::Event], live: Option<&'a oauth::Usage>) -> Inputs<'a> {
        Inputs {
            events,
            now_ms: now_ms(),
            limit: 17_000_000.0,
            weekly_limit: 90_000_000.0,
            session_calibrated: false,
            weekly_calibrated: false,
            live,
        }
    }

    /// Exercises the real pipeline against this machine's session logs.
    #[test]
    fn snapshot_from_local_sessions() {
        let ev = local_events();
        assert!(!ev.is_empty(), "no events parsed");

        let root = usage::projects_dir().unwrap();
        let mut sc = usage::Scanner::default();
        let first = sc.scan(&root).len();
        assert_eq!(first, sc.scan(&root).len(), "incremental rescan changed event count");

        let snap = blocks::build(inputs(&ev, None));
        assert_eq!(snap.source, Source::Estimated);
        for s in &snap.series {
            assert_eq!(s.values.len(), snap.filled_buckets, "series length mismatch");
            for w in s.values.windows(2) {
                assert!(w[1] >= w[0] - 1e-9, "series must not decrease inside a block");
            }
        }
        let tail: f64 = snap.series.iter().filter_map(|s| s.values.last()).sum();
        assert!((tail - snap.used_pct).abs() < 0.05,
            "series tail {tail} disagrees with used_pct {}", snap.used_pct);
    }

    /// A live reading must override the headline number, pin the block to
    /// the reported reset instant, and solve for the real limit so the
    /// plotted curve still lands exactly on that number.
    #[test]
    fn live_usage_overrides_and_derives_the_limit() {
        let ev = local_events();
        let now = now_ms();
        let resets_at = now / 1000 + 3600; // an hour from now
        let live = oauth::Usage {
            five_hour: Some(oauth::Window { pct: 41.2, resets_at }),
            seven_day: Some(oauth::Window { pct: 95.1, resets_at: resets_at + 86_400 }),
            seven_day_opus: None,
        };

        let snap = blocks::build(Inputs { now_ms: now, ..inputs(&ev, Some(&live)) });

        assert_eq!(snap.source, Source::Live);
        assert!((snap.used_pct - 41.2).abs() < 1e-9, "headline must come from the endpoint");
        assert_eq!(snap.block_end_ms, Some(resets_at * 1000));
        assert_eq!(snap.block_start_ms, Some(resets_at * 1000 - blocks::BLOCK_MS));
        assert!((snap.weekly.pct - 95.1).abs() < 1e-9);
        assert_eq!(snap.weekly.resets_at_ms, Some((resets_at + 86_400) * 1000));

        if snap.messages > 0 {
            let derived = snap.derived_limit.expect("limit should be solvable");
            assert!(derived > 0.0);
            let tail: f64 = snap.series.iter().filter_map(|s| s.values.last()).sum();
            assert!((tail - 41.2).abs() < 0.05,
                "curve tail {tail} must land on the reported 41.2%");
        }
    }

    /// A percentage too small to divide by must not produce an absurd
    /// limit; the panel keeps the limit it already had.
    #[test]
    fn tiny_percentages_do_not_derive_a_limit() {
        let ev = local_events();
        let live = oauth::Usage {
            five_hour: Some(oauth::Window { pct: 0.0, resets_at: now_ms() / 1000 + 3600 }),
            seven_day: None,
            seven_day_opus: None,
        };
        let snap = blocks::build(inputs(&ev, Some(&live)));
        assert!(snap.derived_limit.is_none(), "0% must not be divided by");
        assert_eq!(snap.limit_tokens, 17_000_000.0, "falls back to the supplied limit");
    }

    #[test]
    fn calibration_round_trips() {
        let ev = local_events();
        let now = now_ms();
        let Some(start) = blocks::active_block_start(&ev, now) else { return };
        let counted: f64 = ev
            .iter()
            .filter(|e| e.ts_ms >= start && e.ts_ms < start + blocks::BLOCK_MS)
            .map(|e| e.weighted)
            .sum();

        for observed in [12.5_f64, 50.0, 99.0] {
            let solved = counted / (observed / 100.0);
            let snap = blocks::build(Inputs {
                limit: solved,
                session_calibrated: true,
                now_ms: now,
                ..inputs(&ev, None)
            });
            assert_eq!(snap.source, Source::Calibrated);
            assert!((snap.used_pct - observed).abs() < 0.01,
                "calibrating to {observed}% produced {}%", snap.used_pct);
        }
    }

    #[test]
    fn settings_precedence_and_round_trip() {
        let ev = local_events();

        let (limit, _, known, _) = settings::effective(&settings::Settings::default(), &ev);
        assert!(!known);
        assert!(limit > settings::busiest_block(&ev), "provisional limit needs headroom");

        // A solved limit beats the provisional one, and a manual entry
        // beats the solved one.
        let auto = settings::Settings { auto_limit_tokens: Some(30.0), ..Default::default() };
        let (limit, _, known, _) = settings::effective(&auto, &ev);
        assert_eq!((limit, known), (30.0, true));

        let both = settings::Settings { limit_tokens: Some(42.0), ..auto.clone() };
        assert_eq!(settings::effective(&both, &ev).0, 42.0, "manual must win");

        let bad = settings::Settings { limit_tokens: Some(0.0), ..Default::default() };
        let (limit, _, known, _) = settings::effective(&bad, &ev);
        assert!(limit > 0.0 && !known, "a zero limit must fall back, not stick");

        let dir = std::env::temp_dir().join("cum-test-settings");
        let _ = std::fs::remove_dir_all(&dir);
        settings::save(&dir, &both).expect("save");
        assert_eq!(settings::load(&dir).limit_tokens, Some(42.0));
        settings::remember_derived(&dir, Some(123.0), None);
        assert_eq!(settings::load(&dir).auto_limit_tokens, Some(123.0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn labels_are_readable() {
        assert_eq!(blocks::label_for("claude-opus-5"), "Opus 5");
        assert_eq!(blocks::label_for("claude-haiku-4-5-20251001"), "Haiku 4.5");
        assert_eq!(blocks::label_for("claude-sonnet-5"), "Sonnet 5");
    }
}
