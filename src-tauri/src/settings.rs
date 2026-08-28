//! Remembers what only the live usage endpoint can tell us.
//!
//! Nothing on disk records what your plan's ceiling is, but the endpoint
//! reports the percentage spent, and dividing our own token count by it
//! yields the limit. Keeping that value means losing the network drops
//! the panel to the last known real limit rather than to a guess.
//!
//! The reset instants are kept for the same reason and matter just as
//! much. A block boundary usually falls where no message was sent, so
//! the logs cannot find it; forgetting it left the offline axis to be
//! re-guessed from the log alone, hours away from where it really was.
//!
//! There is deliberately no hand-entered fallback: reading a percentage
//! out of `/usage` needs the same token and network the endpoint itself
//! needs, so anything a person could type in, the app could have fetched.

use crate::blocks;
use crate::usage::Event;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const WEEK_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// Headroom over the observed maximum, so an uncalibrated panel does not
/// sit pinned at exactly 100%.
const PROVISIONAL_HEADROOM: f64 = 1.05;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Solved from the live endpoint, so the last known real limit
    /// survives going offline. Unknown fields from older versions -
    /// including the hand-entered limits this replaced - are dropped on
    /// the next write.
    pub auto_limit_tokens: Option<f64>,
    pub auto_weekly_limit_tokens: Option<f64>,
    /// Reset instants as the endpoint reported them, unix ms. Absent
    /// from configs written before they were kept, which simply means
    /// the axis falls back to the log until the next live reading.
    pub auto_block_end_ms: Option<i64>,
    pub auto_weekly_end_ms: Option<i64>,
}

/// What a live reading taught us that outlives it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Derived {
    pub limit: Option<f64>,
    pub weekly_limit: Option<f64>,
    pub block_end_ms: Option<i64>,
    pub weekly_end_ms: Option<i64>,
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

/// Largest weighted total of any single block on record. Grouped by the
/// same rule the panel draws with, so the provisional limit and the
/// curve cannot be measuring different blocks.
pub fn busiest_block(events: &[Event]) -> f64 {
    let mut best = 0.0f64;
    let mut acc = 0.0f64;
    let mut start: Option<i64> = None;
    let mut last: Option<i64> = None;
    for ev in events {
        if blocks::opens_block(start, last, ev.ts_ms) {
            best = best.max(acc);
            acc = 0.0;
            start = Some(ev.ts_ms);
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

/// The limit to render with: the one solved from a previous live
/// reading, else the observed high-water mark plus headroom.
pub fn effective(settings: &Settings, events: &[Event]) -> (f64, f64, bool, bool) {
    let positive = |v: Option<f64>| v.filter(|x| *x > 0.0);
    let session_known = positive(settings.auto_limit_tokens);
    let weekly_known = positive(settings.auto_weekly_limit_tokens);
    let session = session_known.unwrap_or_else(|| (busiest_block(events) * PROVISIONAL_HEADROOM).max(1.0));
    let weekly = weekly_known.unwrap_or_else(|| (busiest_week(events) * PROVISIONAL_HEADROOM).max(1.0));
    (session, weekly, session_known.is_some(), weekly_known.is_some())
}

/// A limit is solved by division, so it jitters a little between polls;
/// only a real move is worth a write. Absent live data, whatever we
/// already knew stands.
fn keep_limit(old: Option<f64>, new: Option<f64>) -> Option<f64> {
    match (old, new) {
        (_, None) => old,
        (None, Some(_)) => new,
        (Some(o), Some(n)) if o <= 0.0 || (n - o).abs() / o > 0.01 => new,
        _ => old,
    }
}

/// Stores what a live reading taught us. This runs on every poll, so it
/// writes only when something actually changed.
pub fn remember_derived(dir: &Path, d: Derived) {
    let old = load(dir);
    let next = Settings {
        auto_limit_tokens: keep_limit(old.auto_limit_tokens, d.limit),
        auto_weekly_limit_tokens: keep_limit(old.auto_weekly_limit_tokens, d.weekly_limit),
        // Reset instants are exact and change only when a block rolls,
        // so any difference is real and none of it needs a threshold.
        auto_block_end_ms: d.block_end_ms.or(old.auto_block_end_ms),
        auto_weekly_end_ms: d.weekly_end_ms.or(old.auto_weekly_end_ms),
    };
    if next != old {
        let _ = save(dir, &next);
    }
}
