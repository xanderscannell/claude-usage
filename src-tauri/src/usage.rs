//! Reads Claude Code's local session logs and turns them into weighted
//! usage events.
//!
//! Session files are append-only JSONL under `~/.claude/projects/`. A
//! resumed or branched session re-logs requests it has already written,
//! so records must be deduplicated on (requestId, message.id) - on real
//! data roughly half of all records are repeats.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Weights that put every token class on one scale, in input-token
/// equivalents, mirroring the published price ratios. Rate limits track
/// compute rather than raw token count, so cache reads must not be
/// counted at par - unweighted, they swamp everything else by ~7x.
pub const W_INPUT: f64 = 1.0;
pub const W_OUTPUT: f64 = 5.0;
pub const W_CACHE_WRITE_5M: f64 = 1.25;
pub const W_CACHE_WRITE_1H: f64 = 2.0;
pub const W_CACHE_READ: f64 = 0.1;

#[derive(Debug, Clone)]
pub struct Event {
    pub ts_ms: i64,
    pub model: String,
    pub weighted: f64,
    pub raw: u64,
    /// Dedup key: (requestId, message.id).
    pub key: (String, String),
}

pub fn projects_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let dir = home.join(".claude").join("projects");
    dir.is_dir().then_some(dir)
}

fn number(v: Option<&serde_json::Value>) -> f64 {
    v.and_then(|x| x.as_f64()).unwrap_or(0.0)
}

/// Parses one JSONL line, returning an event only for assistant records
/// that carry real usage. `<synthetic>` responses never hit the API.
fn parse_line(line: &str) -> Option<Event> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let msg = v.get("message")?;
    let model = msg.get("model")?.as_str()?.to_string();
    if model.is_empty() || model == "<synthetic>" {
        return None;
    }
    let u = msg.get("usage")?;
    if !u.is_object() {
        return None;
    }

    let input = number(u.get("input_tokens"));
    let output = number(u.get("output_tokens"));
    let cache_read = number(u.get("cache_read_input_tokens"));
    let cache_total = number(u.get("cache_creation_input_tokens"));

    // Prefer the ephemeral breakdown so 1h writes are weighted correctly;
    // fall back to the flat total at the 5m rate when it is absent.
    let (cw5, cw1h) = match u.get("cache_creation") {
        Some(c) if c.is_object() => (
            number(c.get("ephemeral_5m_input_tokens")),
            number(c.get("ephemeral_1h_input_tokens")),
        ),
        _ => (cache_total, 0.0),
    };

    let weighted = input * W_INPUT
        + output * W_OUTPUT
        + cw5 * W_CACHE_WRITE_5M
        + cw1h * W_CACHE_WRITE_1H
        + cache_read * W_CACHE_READ;

    let ts = v.get("timestamp")?.as_str()?;
    let ts_ms = chrono::DateTime::parse_from_rfc3339(ts).ok()?.timestamp_millis();

    let request_id = v.get("requestId").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let msg_id = msg.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();

    Some(Event {
        ts_ms,
        model,
        weighted,
        raw: (input + output + cache_total + cache_read) as u64,
        key: (request_id, msg_id),
    })
}

#[derive(Default)]
struct CachedFile {
    len: u64,
    events: Vec<Event>,
}

/// Incremental scanner. Session files only ever grow, so on each poll we
/// re-read just the bytes appended since last time.
#[derive(Default)]
pub struct Scanner {
    files: HashMap<PathBuf, CachedFile>,
}

impl Scanner {
    fn read_from(path: &Path, offset: u64) -> std::io::Result<Vec<Event>> {
        let mut file = File::open(path)?;
        if offset > 0 {
            file.seek(SeekFrom::Start(offset))?;
        }
        let mut events = Vec::new();
        for line in BufReader::new(file).lines() {
            let Ok(line) = line else { continue };
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                if let Some(ev) = parse_line(trimmed) {
                    events.push(ev);
                }
            }
        }
        Ok(events)
    }

    /// Returns every known event, deduplicated, sorted by timestamp.
    pub fn scan(&mut self, root: &Path) -> Vec<Event> {
        let mut seen_paths = Vec::new();

        for entry in walk_jsonl(root) {
            let Ok(meta) = entry.metadata() else { continue };
            let len = meta.len();
            let path = entry.path();
            seen_paths.push(path.clone());

            let cached = self.files.entry(path.clone()).or_default();
            if len == cached.len {
                continue;
            }
            // A shrunken file was rewritten; re-read it whole.
            let from = if len < cached.len {
                cached.events.clear();
                0
            } else {
                cached.len
            };
            // Resume at the last complete line so a partially flushed
            // record is not split across two reads.
            let start = if from == 0 { 0 } else { from };
            if let Ok(mut fresh) = Self::read_from(&path, start) {
                cached.events.append(&mut fresh);
                cached.len = len;
            }
        }

        self.files.retain(|p, _| seen_paths.contains(p));

        let mut seen = std::collections::HashSet::new();
        let mut out: Vec<Event> = Vec::new();
        for ev in self.files.values().flat_map(|f| f.events.iter()) {
            if seen.insert(ev.key.clone()) {
                out.push(ev.clone());
            }
        }
        out.sort_by_key(|e| e.ts_ms);
        out
    }
}

struct Entry(PathBuf);
impl Entry {
    fn metadata(&self) -> std::io::Result<std::fs::Metadata> {
        std::fs::metadata(&self.0)
    }
    fn path(&self) -> PathBuf {
        self.0.clone()
    }
}

fn walk_jsonl(root: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "jsonl") {
                out.push(Entry(p));
            }
        }
    }
    out
}
