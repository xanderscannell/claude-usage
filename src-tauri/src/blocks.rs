//! Groups usage events into Claude's 5-hour limit blocks and builds the
//! snapshot the panel renders.
//!
//! The limit is not a rolling window: a block opens at a message sent
//! outside the previous block, runs five hours from that instant, then
//! resets. So the paper shows one block end to end, the pen sits at now,
//! and the blank paper to its right is the time you have left.
//!
//! Boundaries are not hour aligned. The endpoint reports five-hour resets
//! at ten past as readily as on the hour, and while usage continues each
//! block chains straight on from the last - so the edge usually falls in
//! a silence, where no message marks it. That is why the reset instant is
//! kept across restarts: it is the one fact about the axis the logs
//! cannot yield, and re-guessing it put the axis over two hours out.
//!
//! Two sources feed this. Local session logs give the shape of the curve
//! and the per-model split, which nothing else can. When Claude Code's
//! usage endpoint is reachable it supplies the truth on top: the real
//! percentage, the real reset instant, and - by dividing our own token
//! count by that percentage - the real limit. Nothing is ever entered by
//! hand.

use crate::oauth;
use crate::usage::Event;
use serde::Serialize;

pub const BLOCK_MS: i64 = 5 * 60 * 60 * 1000;
pub const BUCKETS: usize = 60;
const BUCKET_MS: i64 = BLOCK_MS / BUCKETS as i64;
const WEEK_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// Window used to measure the current burn rate.
const BURN_MS: i64 = 30 * 60 * 1000;
/// Below this the reported percentage is too coarse to divide by.
const MIN_PCT_FOR_DERIVATION: f64 = 0.5;

/// Where the headline numbers came from, so the panel can say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Claude Code's usage endpoint: exact.
    Live,
    /// A limit solved from an earlier live reading.
    Remembered,
    /// Scaled to the busiest block on record.
    Estimated,
}

pub struct Inputs<'a> {
    pub events: &'a [Event],
    pub now_ms: i64,
    pub limit: f64,
    pub weekly_limit: f64,
    pub session_known: bool,
    pub weekly_known: bool,
    /// Reset instants kept from an earlier live reading, unix ms. They
    /// pin the axis while they last, and anchor the fallback after.
    pub known_block_end_ms: Option<i64>,
    pub known_weekly_end_ms: Option<i64>,
    pub live: Option<&'a oauth::Usage>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub id: String,
    pub label: String,
    pub weighted: f64,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSeries {
    pub id: String,
    pub label: String,
    pub values: Vec<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Weekly {
    pub source: Source,
    pub pct: f64,
    pub limit_tokens: f64,
    pub weighted: f64,
    /// Unix ms, only ever the endpoint's answer - live, or the one it
    /// last gave us while that is still in the future.
    pub resets_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub source: Source,
    pub limit_tokens: f64,
    /// Set when live data let us solve for the limit; worth persisting.
    #[serde(skip)]
    pub derived_limit: Option<f64>,
    #[serde(skip)]
    pub derived_weekly_limit: Option<f64>,
    /// The reset instants a live reading gave us; worth persisting for
    /// the same reason the limit is.
    #[serde(skip)]
    pub derived_block_end_ms: Option<i64>,
    #[serde(skip)]
    pub derived_weekly_end_ms: Option<i64>,
    /// Null when no block is open - nothing has been sent in five hours.
    pub block_start_ms: Option<i64>,
    pub block_end_ms: Option<i64>,
    pub now_ms: i64,
    pub resets_in_seconds: i64,
    /// Cumulative usage per model as a percentage of the limit, at each
    /// five-minute mark. Buckets after now are omitted: that is paper the
    /// pen has not reached.
    pub series: Vec<ModelSeries>,
    pub filled_buckets: usize,
    pub used_pct: f64,
    pub burn_pct_per_hour: f64,
    pub minutes_to_limit: Option<i64>,
    pub projected_end_pct: f64,
    pub models: Vec<ModelUsage>,
    pub messages: usize,
    pub raw_tokens: u64,
    pub weekly: Weekly,
    /// Why live data is unavailable, when it is not. Never a secret.
    pub live_error: Option<String>,
}

/// True when `ts_ms` falls outside the block that `start` opened, and so
/// opens one of its own. The single definition of where a block ends, so
/// the panel and the high-water mark cannot drift apart on it.
pub fn opens_block(start: Option<i64>, last: Option<i64>, ts_ms: i64) -> bool {
    match (start, last) {
        (None, _) => true,
        (Some(s), Some(l)) => ts_ms >= s + BLOCK_MS || ts_ms - l > BLOCK_MS,
        (Some(s), None) => ts_ms >= s + BLOCK_MS,
    }
}

pub fn label_for(model: &str) -> String {
    let stripped = model.strip_prefix("claude-").unwrap_or(model);
    // Drop a trailing date stamp such as `-20251001`.
    let parts: Vec<&str> = stripped
        .split('-')
        .filter(|p| !(p.len() == 8 && p.chars().all(|c| c.is_ascii_digit())))
        .collect();
    let mut out = Vec::new();
    let mut version = Vec::new();
    for p in parts {
        if p.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            version.push(p.to_string());
        } else {
            let mut c = p.chars();
            if let Some(f) = c.next() {
                out.push(f.to_uppercase().collect::<String>() + c.as_str());
            }
        }
    }
    if !version.is_empty() {
        out.push(version.join("."));
    }
    if out.is_empty() { model.to_string() } else { out.join(" ") }
}

/// Finds the start of the block that is open at `now`, if any, by
/// chaining forward through the log. A block is anchored at the message
/// that opened it, not at the hour containing it: the endpoint reports
/// resets at ten past the hour, so flooring would move the axis by up to
/// an hour on nothing but an assumption.
pub fn active_block_start(events: &[Event], now_ms: i64) -> Option<i64> {
    let mut start: Option<i64> = None;
    let mut last: Option<i64> = None;
    for ev in events {
        if opens_block(start, last, ev.ts_ms) {
            start = Some(ev.ts_ms);
        }
        last = Some(ev.ts_ms);
    }
    start.filter(|s| now_ms < s + BLOCK_MS)
}

/// Where the open block starts when there is no live reading.
///
/// A reset instant the endpoint reported is exact, and it almost never
/// coincides with a message, so it cannot be rediscovered from the logs.
/// While it is still in the future it pins the block outright - offline
/// the axis then matches live to the minute. Once it passes, the chain
/// restarts from it rather than from the top of the log, so a block the
/// server closed hours ago can no longer hold the axis open.
fn offline_block_start(events: &[Event], now_ms: i64, known_end: Option<i64>) -> Option<i64> {
    match known_end {
        // Still inside the block the endpoint reported.
        Some(end) if (end - BLOCK_MS..end).contains(&now_ms) => Some(end - BLOCK_MS),
        // It has passed: only a message after it can open the next block.
        Some(end) if now_ms >= end => {
            let after = events.partition_point(|e| e.ts_ms < end);
            active_block_start(&events[after..], now_ms)
        }
        // Nothing to trust - no reset kept, or one further off than a
        // block can reach, which means a stale write or a moved clock.
        _ => active_block_start(events, now_ms),
    }
}

pub fn build(input: Inputs<'_>) -> Snapshot {
    let Inputs { events, now_ms, live, .. } = input;
    let five = live.and_then(|u| u.five_hour);

    // The endpoint's reset instant pins the block exactly; without it we
    // fall back to the last one it gave us, and only then to the log.
    let block_start = match five {
        Some(w) => Some(w.resets_at * 1000 - BLOCK_MS),
        None => offline_block_start(events, now_ms, input.known_block_end_ms),
    };
    let block_end = block_start.map(|s| s + BLOCK_MS);

    let in_block: Vec<&Event> = match block_start {
        Some(s) => events.iter().filter(|e| e.ts_ms >= s && e.ts_ms < s + BLOCK_MS).collect(),
        None => Vec::new(),
    };

    // Per-model totals, biggest pen first.
    let mut totals: std::collections::HashMap<&str, (f64, u64)> = std::collections::HashMap::new();
    for e in &in_block {
        let slot = totals.entry(e.model.as_str()).or_insert((0.0, 0));
        slot.0 += e.weighted;
        slot.1 += e.raw;
    }
    let mut ranked: Vec<(&str, f64)> = totals.iter().map(|(m, v)| (*m, v.0)).collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let counted: f64 = ranked.iter().map(|r| r.1).sum();

    // Dividing our own count by the reported percentage yields the real
    // limit, so the curve lands exactly on the headline number.
    let derived_limit = five
        .filter(|w| w.pct >= MIN_PCT_FOR_DERIVATION && counted > 0.0)
        .map(|w| counted / (w.pct / 100.0));

    let limit = derived_limit.unwrap_or(input.limit).max(1.0);
    let source = match (five.is_some(), input.session_known) {
        (true, _) => Source::Live,
        (false, true) => Source::Remembered,
        (false, false) => Source::Estimated,
    };
    let used_pct = match five {
        Some(w) => w.pct,
        None => counted / limit * 100.0,
    };

    // Cumulative series, one pen per model, cut off at the pen's position.
    let filled = match block_start {
        Some(s) => (((now_ms - s) as f64 / BUCKET_MS as f64).ceil() as i64).clamp(0, BUCKETS as i64) as usize,
        None => 0,
    };
    let series: Vec<ModelSeries> = ranked
        .iter()
        .map(|(model, _)| {
            let mut values = vec![0.0; filled];
            if let Some(s) = block_start {
                let mut acc = 0.0;
                let mut idx = 0usize;
                let mut evs: Vec<&&Event> = in_block.iter().filter(|e| e.model == *model).collect();
                evs.sort_by_key(|e| e.ts_ms);
                for e in evs {
                    let b = (((e.ts_ms - s) / BUCKET_MS) as usize).min(filled.saturating_sub(1));
                    while idx < b && idx < filled {
                        values[idx] = acc;
                        idx += 1;
                    }
                    acc += e.weighted;
                }
                while idx < filled {
                    values[idx] = acc;
                    idx += 1;
                }
            }
            for v in values.iter_mut() {
                *v = *v / limit * 100.0;
            }
            ModelSeries { id: model.to_string(), label: label_for(model), values }
        })
        .collect();

    // Burn measured over the last half hour of real activity.
    let burn_weighted: f64 = events
        .iter()
        .filter(|e| e.ts_ms > now_ms - BURN_MS && e.ts_ms <= now_ms)
        .map(|e| e.weighted)
        .sum();
    let burn_pct_per_hour = burn_weighted / limit * 100.0 * (60.0 * 60.0 * 1000.0 / BURN_MS as f64);

    // Inside a block nothing ages out, so the projection is linear.
    let remaining_min = block_end.map(|e| (e - now_ms).max(0) / 60_000).unwrap_or(0);
    let per_min = burn_pct_per_hour / 60.0;
    let minutes_to_limit = if per_min > 0.0 && used_pct < 100.0 {
        let m = ((100.0 - used_pct) / per_min).ceil() as i64;
        (m <= remaining_min).then_some(m)
    } else {
        None
    };
    let projected_end_pct = (used_pct + per_min * remaining_min as f64).min(999.0);

    let weekly_weighted: f64 = events
        .iter()
        .filter(|e| e.ts_ms > now_ms - WEEK_MS)
        .map(|e| e.weighted)
        .sum();
    let seven = live.and_then(|u| u.seven_day);
    let derived_weekly_limit = seven
        .filter(|w| w.pct >= MIN_PCT_FOR_DERIVATION && weekly_weighted > 0.0)
        .map(|w| weekly_weighted / (w.pct / 100.0));
    let weekly_limit = derived_weekly_limit.unwrap_or(input.weekly_limit).max(1.0);

    Snapshot {
        source,
        limit_tokens: limit,
        derived_limit,
        derived_weekly_limit,
        derived_block_end_ms: five.map(|w| w.resets_at * 1000),
        derived_weekly_end_ms: seven.map(|w| w.resets_at * 1000),
        block_start_ms: block_start,
        block_end_ms: block_end,
        now_ms,
        resets_in_seconds: block_end.map(|e| (e - now_ms).max(0) / 1000).unwrap_or(0),
        series,
        filled_buckets: filled,
        used_pct,
        burn_pct_per_hour,
        minutes_to_limit,
        projected_end_pct,
        models: ranked
            .iter()
            .map(|(m, w)| ModelUsage {
                id: m.to_string(),
                label: label_for(m),
                weighted: *w,
                pct: w / limit * 100.0,
            })
            .collect(),
        messages: in_block.len(),
        raw_tokens: totals.values().map(|v| v.1).sum(),
        weekly: Weekly {
            source: match (seven.is_some(), input.weekly_known) {
                (true, _) => Source::Live,
                (false, true) => Source::Remembered,
                (false, false) => Source::Estimated,
            },
            pct: match seven {
                Some(w) => w.pct,
                None => weekly_weighted / weekly_limit * 100.0,
            },
            limit_tokens: weekly_limit,
            weighted: weekly_weighted,
            // A remembered weekly reset is still the endpoint's answer,
            // not a guess, so it stands until it passes.
            resets_at_ms: seven
                .map(|w| w.resets_at * 1000)
                .or(input.known_weekly_end_ms.filter(|e| *e > now_ms)),
        },
        live_error: None,
    }
}
