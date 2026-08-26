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
    /// Overrides the retry window when a failure asked us to wait.
    retry_after_ms: Option<i64>,
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
        let ttl = cache
            .retry_after_ms
            .unwrap_or(if cache.usage.is_some() { LIVE_TTL_MS } else { LIVE_RETRY_MS });
        if cache.fetched_at_ms > 0 && age < ttl {
            return (cache.usage.clone(), cache.error.clone());
        }
    }

    let fetched = tauri::async_runtime::spawn_blocking(oauth::fetch).await;
    let (usage, error, retry_after_ms) = match fetched {
        Ok(Ok(u)) => (Some(u), None, None),
        Ok(Err(e)) => {
            let wait = e.retry_after_secs().map(|s| s * 1000);
            (None, Some(e.to_string()), wait)
        }
        Err(e) => (None, Some(format!("Live usage task failed: {e}")), None),
    };

    if let Ok(mut cache) = state.live.lock() {
        cache.fetched_at_ms = now;
        cache.usage = usage.clone();
        cache.error = error.clone();
        cache.retry_after_ms = retry_after_ms;
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
    let (limit, weekly_limit, known_s, known_w) = settings::effective(&cfg, events);

    let mut snap = blocks::build(blocks::Inputs {
        events,
        now_ms: now_ms(),
        limit,
        weekly_limit,
        session_known: known_s,
        weekly_known: known_w,
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(App::default())
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            *app.state::<App>().config_dir.lock().unwrap() = dir;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_snapshot])
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
            session_known: false,
            weekly_known: false,
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

    /// Offline, a limit solved from an earlier live reading must still
    /// reproduce the percentage it was solved from.
    #[test]
    fn a_remembered_limit_survives_going_offline() {
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
                session_known: true,
                now_ms: now,
                ..inputs(&ev, None)
            });
            assert_eq!(snap.source, Source::Remembered);
            assert!((snap.used_pct - observed).abs() < 0.01,
                "a limit solved at {observed}% produced {}%", snap.used_pct);
        }
    }

    #[test]
    fn settings_precedence_and_round_trip() {
        let ev = local_events();

        let (limit, _, known, _) = settings::effective(&settings::Settings::default(), &ev);
        assert!(!known);
        assert!(limit > settings::busiest_block(&ev), "provisional limit needs headroom");

        // A solved limit beats the provisional one.
        let auto = settings::Settings { auto_limit_tokens: Some(30.0), ..Default::default() };
        let (limit, _, known, _) = settings::effective(&auto, &ev);
        assert_eq!((limit, known), (30.0, true));

        let bad = settings::Settings { auto_limit_tokens: Some(0.0), ..Default::default() };
        let (limit, _, known, _) = settings::effective(&bad, &ev);
        assert!(limit > 0.0 && !known, "a zero limit must fall back, not stick");

        let dir = std::env::temp_dir().join("cum-test-settings");
        let _ = std::fs::remove_dir_all(&dir);
        settings::remember_derived(&dir, Some(123.0), Some(456.0));
        let back = settings::load(&dir);
        assert_eq!((back.auto_limit_tokens, back.auto_weekly_limit_tokens),
            (Some(123.0), Some(456.0)));

        // A config written by the old hand-calibration build must load
        // without error; its fields are simply dropped.
        std::fs::write(dir.join("calibration.json"),
            r#"{"limitTokens":42.0,"weeklyLimitTokens":7.0,"calibratedAtMs":1,"autoLimitTokens":9.0}"#).unwrap();
        let legacy = settings::load(&dir);
        assert_eq!(legacy.auto_limit_tokens, Some(9.0), "legacy config must still load");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn labels_are_readable() {
        assert_eq!(blocks::label_for("claude-opus-5"), "Opus 5");
        assert_eq!(blocks::label_for("claude-haiku-4-5-20251001"), "Haiku 4.5");
        assert_eq!(blocks::label_for("claude-sonnet-5"), "Sonnet 5");
    }
}

#[cfg(test)]
mod diagnose {
    use super::*;
    use blocks::Inputs;

    /// Prints every time-related figure the panel derives, so a
    /// disagreement between two readouts can be traced to its source.
    /// `cargo test -- --ignored --nocapture diagnose`
    #[test]
    #[ignore]
    fn time_readouts() {
        let root = usage::projects_dir().expect("projects dir");
        let ev = usage::Scanner::default().scan(&root);
        let live = oauth::fetch().ok();
        let now = now_ms();
        let snap = blocks::build(Inputs {
            events: &ev,
            now_ms: now,
            limit: 1.0,
            weekly_limit: 1.0,
            session_known: false,
            weekly_known: false,
            live: live.as_ref(),
        });

        let hm = |ms: i64| {
            let d = (ms - now) as f64 / 3_600_000.0;
            format!("{d:+.2}h")
        };
        eprintln!("source          = {:?}", snap.source);
        eprintln!("now             = {now}");
        eprintln!("block_start     = {:?} {}", snap.block_start_ms, snap.block_start_ms.map(hm).unwrap_or_default());
        eprintln!("block_end       = {:?} {}", snap.block_end_ms, snap.block_end_ms.map(hm).unwrap_or_default());
        eprintln!("resets_in       = {} s = {:.2}h", snap.resets_in_seconds, snap.resets_in_seconds as f64 / 3600.0);
        eprintln!("used_pct        = {:.2}", snap.used_pct);
        eprintln!("burn_pct_per_hr = {:.2}", snap.burn_pct_per_hour);
        eprintln!("minutes_to_limit= {:?}", snap.minutes_to_limit);
        eprintln!("projected_end   = {:.2}", snap.projected_end_pct);
        eprintln!("filled_buckets  = {} of {}", snap.filled_buckets, blocks::BUCKETS);

        // What the chart draws for the crossing marker.
        if let (Some(end), true) = (snap.block_end_ms, snap.projected_end_pct > 100.0) {
            let t = (100.0 - snap.used_pct) / (snap.projected_end_pct - snap.used_pct);
            let ct = now as f64 + t * (end - now) as f64;
            eprintln!("chart crossing  = {:+.2}h from now (t={t:.4})", (ct - now as f64) / 3_600_000.0);
            eprintln!("readout says    = {:?} min = {:.2}h",
                snap.minutes_to_limit, snap.minutes_to_limit.unwrap_or(0) as f64 / 60.0);
        }
    }
}
