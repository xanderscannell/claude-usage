# Claude Usage

A fixed 800×480 desk panel that answers one question at a glance: **how much of the
current 5-hour block is gone, and will I hit the wall before it resets?**

It reads Claude Code's own session logs. No API keys, no network, no credentials.

```bash
npm install
npm run tauri dev     # the panel
npm run dev           # frontend only, on synthetic data
cd src-tauri && cargo test
```

## Where the numbers come from

Claude Code appends every exchange to JSONL under `~/.claude/projects/`. Each
assistant record carries the model, a timestamp, and the full token breakdown.
Two things about that data are easy to get wrong, and both are handled:

- **Half the records are duplicates.** Resuming or branching a session re-logs
  requests it already wrote — on this machine, 2,450 of 4,896. Events are
  deduplicated on `(requestId, message.id)`; skip that and every number roughly
  doubles.
- **Raw token counts are meaningless.** Cache reads run about 7× everything else
  combined, so an unweighted total is really just a cache-read counter. Tokens are
  weighted into input-equivalents at the published price ratios — output 5×, cache
  writes 1.25×/2×, cache reads 0.1× — because limits track compute, not volume.

Files are re-read incrementally: session logs only grow, so each poll parses just
the bytes appended since the last one.

## Blocks, not a rolling window

The 5-hour limit is not a rolling window. A block opens at your first message
(floored to the hour), runs five hours, then resets — which is why `/usage` can
name a reset time. So the paper shows **one block end to end**: the pen sits at
now, and the blank paper to its right is the time you have left. The dashed line
is where the current burn rate lands you, and if it crosses the ceiling the
recorder marks the clock time it happens.

## Calibration

Nothing local records your plan's ceiling, and no public API exposes it for a
subscription. So the panel counts your tokens exactly and asks you once what 100%
means: run `/usage` in Claude Code, click **Calibrate**, enter the percentage it
shows. The limit is solved from the tokens counted over that same block and stored
in the app config directory.

Until then it scales to your busiest block on record, which is useful immediately
but reads as "relative to your own high-water mark" — the title rail says so.

## Design

A rolling-window recorder was the wrong metaphor, but a chart recorder was the
right one. The rules that carry information:

- **Oxide red belongs to the scale** — grid, limit line, over-range band, and the
  projected crossing. A model is never red. If something on the paper is red, it
  is telling you about the ceiling.
- **Models are ink pens.** Graphite stacks at the baseline so it can never fuse
  with the graphite total pen riding the top. Each channel signs its own boundary
  at full ink.
- **The total is its own pen and always writes.** Toggling a channel lifts its
  wash but leaves the total line, so the gap you open is that model's contribution.
- **One pen means no legend.** A single model's percentage *is* the hero number,
  so the legend collapses and the paper takes the space.

Type is Archivo at width 125 for readouts and Martian Mono for silkscreen labels.
One motion moment on load; `prefers-reduced-motion` turns it all off.

## Layout

| | |
| --- | --- |
| [src/chart.js](src/chart.js) | the paper — scale, pens, projection |
| [src/main.js](src/main.js) | polling, readouts, calibration sheet |
| [src/api.js](src/api.js) | IPC, with a synthetic fallback for browser dev |
| [src-tauri/src/usage.rs](src-tauri/src/usage.rs) | log scanning, dedup, weighting |
| [src-tauri/src/blocks.rs](src-tauri/src/blocks.rs) | block detection, snapshot |
| [src-tauri/src/settings.rs](src-tauri/src/settings.rs) | calibration store |

## Notes

- Window chrome is drawn by the frontend (`decorations: false`), so the title rail
  is the drag region and carries its own minimize/close.
- Window controls use `withGlobalTauri`, so there is no JS dependency on Tauri and
  the panel still runs in a plain browser.
- Fonts come from Google Fonts. Vendor them into `src/` before shipping so the
  panel renders offline.
- Weekly usage is a rolling 7-day total, labelled `7-DAY`. Anthropic's weekly limit
  resets on a schedule this data does not record, so no reset time is claimed.
