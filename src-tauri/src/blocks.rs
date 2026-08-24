//! Groups usage events into Claude's 5-hour limit blocks and builds the
//! snapshot the panel renders.
//!
//! The limit is not a rolling window: a block opens at your first
//! message (floored to the hour), runs five hours, then resets. So the
//! paper shows one block end to end, the pen sits at now, and the blank
//! paper to its right is the time you have left.

use crate::usage::Event;
use serde::Serialize;

pub const BLOCK_MS: i64 = 5 * 60 * 60 * 1000;
pub const BUCKETS: usize = 60;
const BUCKET_MS: i64 = BLOCK_MS / BUCKETS as i64;
const WEEK_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// Window used to measure the current burn rate.
const BURN_MS: i64 = 30 * 60 * 1000;

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
pub struct Snapshot {
    pub calibrated: bool,
    pub limit_tokens: f64,
    /// Null when no block is open - nothing has been sent in five hours.
    pub block_start_ms: Option<i64>,
    pub block_end_ms: Option<i64>,
    pub now_ms: i64,
    pub resets_in_seconds: i64,
    /// Cumulative weighted usage per model, as a percentage of the
    /// limit, at each five-minute mark. Buckets after now are omitted:
    /// that is paper the pen has not reached.
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
    pub calibrated: bool,
    pub pct: f64,
    pub limit_tokens: f64,
    pub weighted: f64,
}

fn floor_to_hour(ts_ms: i64) -> i64 {
    const HOUR: i64 = 60 * 60 * 1000;
    ts_ms - ts_ms.rem_euclid(HOUR)
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
            match c.next() {
                Some(f) => out.push(f.to_uppercase().collect::<String>() + c.as_str()),
                None => {}
            }
        }
    }
    if !version.is_empty() {
        out.push(version.join("."));
    }
    if out.is_empty() { model.to_string() } else { out.join(" ") }
}

/// Finds the start of the block that is open at `now`, if any.
pub fn active_block_start(events: &[Event], now_ms: i64) -> Option<i64> {
    let mut start: Option<i64> = None;
    let mut last: Option<i64> = None;
    for ev in events {
        let opens_new = match (start, last) {
            (None, _) => true,
            (Some(s), Some(l)) => ev.ts_ms >= s + BLOCK_MS || ev.ts_ms - l > BLOCK_MS,
            (Some(s), None) => ev.ts_ms >= s + BLOCK_MS,
        };
        if opens_new {
            start = Some(floor_to_hour(ev.ts_ms));
        }
        last = Some(ev.ts_ms);
    }
    start.filter(|s| now_ms < s + BLOCK_MS)
}

pub fn build(events: &[Event], now_ms: i64, limit: f64, weekly_limit: f64, calibrated: (bool, bool)) -> Snapshot {
    let block_start = active_block_start(events, now_ms);
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

    let total_weighted: f64 = ranked.iter().map(|r| r.1).sum();
    let used_pct = if limit > 0.0 { total_weighted / limit * 100.0 } else { 0.0 };

    // Cumulative series, one pen per model, cut off at the pen's position.
    let filled = match block_start {
        Some(s) => (((now_ms - s) as f64 / BUCKET_MS as f64).ceil() as usize).clamp(0, BUCKETS),
        None => 0,
    };
    let series = ranked
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
            if limit > 0.0 {
                for v in values.iter_mut() {
                    *v = *v / limit * 100.0;
                }
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
    let burn_pct_per_hour = if limit > 0.0 {
        burn_weighted / limit * 100.0 * (60.0 * 60.0 * 1000.0 / BURN_MS as f64)
    } else {
        0.0
    };

    // Inside a block nothing ages out, so the projection is linear.
    let remaining_min = block_end.map(|e| ((e - now_ms).max(0)) / 60_000).unwrap_or(0);
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

    Snapshot {
        calibrated: calibrated.0,
        limit_tokens: limit,
        block_start_ms: block_start,
        block_end_ms: block_end,
        now_ms,
        resets_in_seconds: block_end.map(|e| ((e - now_ms).max(0)) / 1000).unwrap_or(0),
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
                pct: if limit > 0.0 { w / limit * 100.0 } else { 0.0 },
            })
            .collect(),
        messages: in_block.len(),
        raw_tokens: totals.values().map(|v| v.1).sum(),
        weekly: Weekly {
            calibrated: calibrated.1,
            pct: if weekly_limit > 0.0 { weekly_weighted / weekly_limit * 100.0 } else { 0.0 },
            limit_tokens: weekly_limit,
            weighted: weekly_weighted,
        },
    }
}
