mod align;
mod blocks;
mod macros;
mod oauth;
mod settings;
mod system;
mod touch;
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
        known_block_end_ms: cfg.auto_block_end_ms,
        known_weekly_end_ms: cfg.auto_weekly_end_ms,
        live,
    });

    // Keep the solved limit and the reset instants, so the panel stays
    // accurate offline in both the number and the axis it is plotted on.
    settings::remember_derived(&dir, settings::Derived {
        limit: snap.derived_limit,
        weekly_limit: snap.derived_weekly_limit,
        block_end_ms: snap.derived_block_end_ms,
        weekly_end_ms: snap.derived_weekly_end_ms,
    });
    snap.live_error = live_error;
    Ok(snap)
}

#[tauri::command]
async fn get_snapshot(state: State<'_, App>) -> Result<Snapshot, String> {
    let events = read_events(&state)?;
    let (live, live_error) = live_usage(&state).await;
    assemble(&state, &events, live.as_ref(), live_error)
}

#[tauri::command]
fn get_macros(state: State<App>) -> Result<macros::View, String> {
    Ok(macros::view(&config_dir(&state)?))
}

/// Runs the button at `index`, if it still carries `label`. Only ever what
/// macros.json says; the page cannot name a command of its own.
#[tauri::command]
async fn run_macro(state: State<'_, App>, index: usize, label: String) -> Result<(), String> {
    let dir = config_dir(&state)?;
    tauri::async_runtime::spawn_blocking(move || macros::run(&dir, index, &label))
        .await
        .map_err(|e| e.to_string())?
}

/// A touch on the panel ended: put the mouse back where it was.
#[tauri::command]
fn touch_ended() {
    touch::restore();
}

/// Sends the window back to the panel it belongs on. False means that
/// display is not connected, so nothing moved.
#[tauri::command]
fn align_window(window: tauri::WebviewWindow) -> bool {
    align::align(&window)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(App::default())
        .setup(|app| {
            // Placed before anything fallible, so a bad config dir can
            // never leave the window hidden.
            if let Some(win) = app.get_webview_window("main") {
                align::align(&win);
                let _ = win.show();
            }
            system::spawn(app.handle().clone());
            touch::start();
            let dir = app.path().app_config_dir()?;
            *app.state::<App>().config_dir.lock().unwrap() = dir;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_snapshot, align_window, get_macros, run_macro, touch_ended])
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
            known_block_end_ms: None,
            known_weekly_end_ms: None,
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

    fn ev_at(ts_ms: i64) -> usage::Event {
        usage::Event {
            ts_ms,
            model: "claude-opus-5".into(),
            weighted: 1000.0,
            raw: 1000,
            key: (format!("r{ts_ms}"), format!("m{ts_ms}")),
        }
    }

    /// A block is anchored at the message that opened it. Flooring to the
    /// hour was an assumption the endpoint disproves - it reports
    /// five-hour resets at ten past - and it moved the axis by up to an
    /// hour every time the panel fell back to the log.
    #[test]
    fn blocks_are_not_floored_to_the_hour() {
        let start = 1_787_947_800_000; // 20:10:00Z, ten past the hour
        let ev = [ev_at(start), ev_at(start + 60_000)];
        assert_eq!(
            blocks::active_block_start(&ev, start + 60 * 60_000),
            Some(start),
            "the block must open where the message did"
        );
    }

    /// The whole point of remembering the reset: offline, the axis has to
    /// land where live said it did. Nothing in the log marks that
    /// boundary, so without this it is re-guessed and drifts.
    #[test]
    fn a_remembered_reset_pins_the_offline_axis() {
        let ev = local_events();
        let now = now_ms();
        let resets_at = now / 1000 + 3600;
        let live = oauth::Usage {
            five_hour: Some(oauth::Window { pct: 41.2, resets_at }),
            seven_day: Some(oauth::Window { pct: 95.1, resets_at: resets_at + 86_400 }),
            seven_day_opus: None,
        };

        let online = blocks::build(Inputs { now_ms: now, ..inputs(&ev, Some(&live)) });
        let offline = blocks::build(Inputs {
            now_ms: now,
            known_block_end_ms: online.derived_block_end_ms,
            known_weekly_end_ms: online.derived_weekly_end_ms,
            session_known: true,
            ..inputs(&ev, None)
        });

        assert_eq!(offline.source, Source::Remembered);
        assert_eq!(offline.block_start_ms, online.block_start_ms,
            "offline must open the block where live did");
        assert_eq!(offline.block_end_ms, online.block_end_ms);
        assert_eq!(offline.resets_in_seconds, online.resets_in_seconds);
        assert_eq!(offline.filled_buckets, online.filled_buckets);
        assert_eq!(offline.weekly.resets_at_ms, online.weekly.resets_at_ms,
            "a weekly reset the endpoint gave us is not a guess either");
    }

    /// Once the remembered block has expired the chain has to restart
    /// from it. Chaining from the top of the log instead let a block the
    /// server closed hours ago keep holding the axis open.
    #[test]
    fn an_expired_remembered_reset_anchors_the_next_block() {
        let end = 1_787_947_800_000;
        let opened = end + 10 * 60_000;
        let ev = [ev_at(end - 3 * 60 * 60_000), ev_at(opened), ev_at(opened + 60_000)];
        let now = end + 60 * 60_000;

        let stale = blocks::build(Inputs { now_ms: now, ..inputs(&ev, None) });
        assert_eq!(stale.block_start_ms, Some(end - 3 * 60 * 60_000),
            "without the reset the log alone anchors the block too early");

        let anchored = blocks::build(Inputs {
            now_ms: now,
            known_block_end_ms: Some(end),
            ..inputs(&ev, None)
        });
        assert_eq!(anchored.block_start_ms, Some(opened),
            "only a message after the reset can open the next block");
        assert_eq!(anchored.messages, 2, "usage before the reset must not be counted");
    }

    /// A reset still in the future pins the block whether or not anything
    /// has been sent in it, exactly as a live reading would.
    #[test]
    fn a_future_remembered_reset_holds_an_empty_block_open() {
        let now = now_ms();
        let end = now + 2 * 60 * 60_000;
        let snap = blocks::build(Inputs {
            now_ms: now,
            known_block_end_ms: Some(end),
            ..inputs(&[], None)
        });
        assert_eq!(snap.block_start_ms, Some(end - blocks::BLOCK_MS));
        assert_eq!(snap.block_end_ms, Some(end));
    }

    /// A reset so old the block cannot still be running must not be
    /// trusted, nor one from a clock that has jumped backwards.
    #[test]
    fn an_impossible_remembered_reset_falls_back_to_the_log() {
        let now = now_ms();
        let ev = [ev_at(now - 30 * 60_000)];
        let future = blocks::build(Inputs {
            now_ms: now,
            known_block_end_ms: Some(now + 9 * 60 * 60_000),
            ..inputs(&ev, None)
        });
        assert_eq!(future.block_start_ms, Some(now - 30 * 60_000),
            "a reset more than a block away cannot be the open one");
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
        settings::remember_derived(&dir, settings::Derived {
            limit: Some(123.0),
            weekly_limit: Some(456.0),
            block_end_ms: Some(789),
            weekly_end_ms: Some(1011),
        });
        let back = settings::load(&dir);
        assert_eq!((back.auto_limit_tokens, back.auto_weekly_limit_tokens),
            (Some(123.0), Some(456.0)));
        assert_eq!((back.auto_block_end_ms, back.auto_weekly_end_ms),
            (Some(789), Some(1011)), "reset instants must survive the round trip");

        // A poll with no live reading must not erase what we already knew.
        settings::remember_derived(&dir, settings::Derived::default());
        let kept = settings::load(&dir);
        assert_eq!(kept, back, "an offline poll must leave the config alone");

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

    /// Prints every time-related figure the panel derives, and then what
    /// the offline path makes of the same moment, so a disagreement
    /// between two readouts can be traced to its source.
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
            known_block_end_ms: None,
            known_weekly_end_ms: None,
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

        // The same moment as the panel would draw it with no network.
        // Both offline axes are printed: with the reset we remember, and
        // with the log alone. The gap between them is what forgetting it
        // used to cost.
        let offline = |known| blocks::build(Inputs {
            events: &ev,
            now_ms: now,
            limit: 1.0,
            weekly_limit: 1.0,
            session_known: false,
            weekly_known: false,
            known_block_end_ms: known,
            known_weekly_end_ms: None,
            live: None,
        });
        for (name, known) in [("remembered", snap.derived_block_end_ms), ("log only", None)] {
            let off = offline(known);
            let drift = match (off.block_start_ms, snap.block_start_ms) {
                (Some(o), Some(l)) => format!("{:+.1} min from live", (o - l) as f64 / 60_000.0),
                _ => "no block".into(),
            };
            eprintln!("offline ({name:<11}) start = {:?} {} | {}",
                off.block_start_ms, off.block_start_ms.map(hm).unwrap_or_default(), drift);
        }

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
