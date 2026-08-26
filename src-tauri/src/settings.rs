//! Calibration store.
//!
//! Nothing local records what your plan's ceiling actually is, so the
//! user reads the real percentage out of Claude Code's `/usage` once and
//! we solve for the limit from the tokens we counted over the same
//! period. Until then we fall back to the busiest block on record, which
//! makes the panel useful immediately but reads as "relative to your own
//! high-water mark" rather than a true plan limit.

use crate::blocks::BLOCK_MS;
use crate::usage::Event;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const WEEK_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// Headroom over the observed maximum, so an uncalibrated panel does not
/// sit pinned at exactly 100%.
const PROVISIONAL_HEADROOM: f64 = 1.05;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Entered by hand from `/usage`.
    pub limit_tokens: Option<f64>,
    pub weekly_limit_tokens: Option<f64>,
    /// Solved from the live endpoint, so the last known real limit
    /// survives going offline. Manual values still win.
    pub auto_limit_tokens: Option<f64>,
    pub auto_weekly_limit_tokens: Option<f64>,
    pub calibrated_at_ms: Option<i64>,
}

fn path(dir: &Path) -> PathBuf {
    dir.join("calibration.json")
}

pub fn load(dir: &Path) -> Settings {
    std::fs::read_to_string(path(dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, s: &Settings) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(s).unwrap_or_else(|_| "{}".into());
    std::fs::write(path(dir), json)
}

/// Largest weighted total of any single block on record.
pub fn busiest_block(events: &[Event]) -> f64 {
    const HOUR: i64 = 60 * 60 * 1000;
    let mut best = 0.0f64;
    let mut acc = 0.0f64;
    let mut start: Option<i64> = None;
    let mut last: Option<i64> = None;
    for ev in events {
        let opens_new = match (start, last) {
            (None, _) => true,
            (Some(s), Some(l)) => ev.ts_ms >= s + BLOCK_MS || ev.ts_ms - l > BLOCK_MS,
            (Some(s), None) => ev.ts_ms >= s + BLOCK_MS,
        };
        if opens_new {
            best = best.max(acc);
            acc = 0.0;
            start = Some(ev.ts_ms - ev.ts_ms.rem_euclid(HOUR));
        }
        acc += ev.weighted;
        last = Some(ev.ts_ms);
    }
    best.max(acc)
}

/// Heaviest any seven-day stretch has ever been.
pub fn busiest_week(events: &[Event]) -> f64 {
    let mut best = 0.0f64;
    let mut acc = 0.0f64;
    let mut lo = 0usize;
    for hi in 0..events.len() {
        acc += events[hi].weighted;
        while events[hi].ts_ms - events[lo].ts_ms > WEEK_MS {
            acc -= events[lo].weighted;
            lo += 1;
        }
        best = best.max(acc);
    }
    best
}

/// The limit to render with, in order of trust: entered by hand, then
/// solved from a previous live reading, then the observed high-water
/// mark plus headroom.
pub fn effective(settings: &Settings, events: &[Event]) -> (f64, f64, bool, bool) {
    let positive = |v: Option<f64>| v.filter(|x| *x > 0.0);
    let session_known = positive(settings.limit_tokens).or(positive(settings.auto_limit_tokens));
    let weekly_known = positive(settings.weekly_limit_tokens).or(positive(settings.auto_weekly_limit_tokens));
    let session = session_known.unwrap_or_else(|| (busiest_block(events) * PROVISIONAL_HEADROOM).max(1.0));
    let weekly = weekly_known.unwrap_or_else(|| (busiest_week(events) * PROVISIONAL_HEADROOM).max(1.0));
    (session, weekly, session_known.is_some(), weekly_known.is_some())
}

/// Stores a limit solved from live data, but only when it has moved
/// enough to matter - this runs on every poll.
pub fn remember_derived(dir: &Path, session: Option<f64>, weekly: Option<f64>) {
    let mut cfg = load(dir);
    let moved = |old: Option<f64>, new: Option<f64>| match (old, new) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(o), Some(n)) => o <= 0.0 || (n - o).abs() / o > 0.01,
    };
    let mut dirty = false;
    if moved(cfg.auto_limit_tokens, session) {
        cfg.auto_limit_tokens = session;
        dirty = true;
    }
    if moved(cfg.auto_weekly_limit_tokens, weekly) {
        cfg.auto_weekly_limit_tokens = weekly;
        dirty = true;
    }
    if dirty {
        let _ = save(dir, &cfg);
    }
}
