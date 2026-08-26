# Claude Usage

A fixed 800×480 desk panel that answers one question at a glance: **how much of the
current 5-hour block is gone, and will I hit the wall before it resets?**

It reads Claude Code's own session logs for the shape of the curve, and Claude
Code's own usage endpoint for the truth on top. No API keys, no configuration, and
it still works with the network off.

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

## Where 100% comes from

Nothing on disk records your plan's ceiling. The endpoint reports the percentage
spent, so dividing our own token count by it solves for the limit — which means
**there is nothing to calibrate**. Three sources, in order of trust; the title rail
and footer always say which is in play.

**Live.** Claude Code renders `/usage` by calling `GET /api/oauth/usage` with the
OAuth token in `~/.claude/.credentials.json`; the panel reads the same endpoint.
True percentage, true reset instant, solved limit.

**Remembered.** The solved limit is kept, so losing the network drops the panel to
the last known real limit rather than to a guess.

**Estimated.** Before any live reading has ever succeeded, it scales to the
busiest block on record. On this machine that guessed 17.1M against a real 6.31M —
a 2.7x error — so treat it as a shape, not a number. The title rail says so.

There is deliberately **no hand-entered fallback**. Reading a percentage out of
`/usage` needs the same token and the same network the endpoint itself needs, so
anything a person could type in, the app could have fetched. A calibration form
would only ever be a slower path to a number already in reach.

Constraints on the credential path:

- **Read only.** The credentials file is never written. Refresh tokens normally
  rotate, so refreshing here without persisting could invalidate the CLI's own
  login. An expired token is reported, not repaired — run any Claude Code command
  and it refreshes itself.
- **The token goes to one host** and is never logged, returned through a command,
  or put in an error message. A failed parse names only the windows we looked for.
- **Every failure is soft**, cached for a minute on success and two on failure.
  The endpoint is undocumented: it can change or vanish, and the panel has to keep
  working from local logs when it does.
- `HTTPS_PROXY` and friends are honoured, so a proxied network does not become the
  one case where `/usage` works and the panel does not.

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
| [src/main.js](src/main.js) | polling, readouts, status |
| [src/api.js](src/api.js) | IPC, with a synthetic fallback for browser dev |
| [src-tauri/src/usage.rs](src-tauri/src/usage.rs) | log scanning, dedup, weighting |
| [src-tauri/src/blocks.rs](src-tauri/src/blocks.rs) | block detection, snapshot |
| [src-tauri/src/oauth.rs](src-tauri/src/oauth.rs) | live usage endpoint |
| [src-tauri/src/settings.rs](src-tauri/src/settings.rs) | remembers the solved limit |

## Notes

- The webview runs under a strict CSP: `script-src 'self'`, `object-src 'none'`,
  and network limited to the IPC channel plus Google Fonts. `style-src` still
  allows `'unsafe-inline'` because Vite injects `<style>` blocks in dev; vendoring
  the fonts would let that come out too. `freezePrototype` is on, and the app is
  granted no filesystem, shell, or HTTP plugin permissions — only window
  close/minimize/drag.
- Window chrome is drawn by the frontend (`decorations: false`), so the title rail
  is the drag region and carries its own minimize/close.
- Window controls use `withGlobalTauri`, so there is no JS dependency on Tauri and
  the panel still runs in a plain browser.
- Fonts come from Google Fonts. Vendor them into `src/` before shipping so the
  panel renders offline.
- Weekly usage comes from the endpoint's `seven_day` window, with its real reset
  time. Without live data it falls back to a rolling 7-day total labelled `7-DAY`
  and claims no reset schedule, because the logs do not record one.
- `cargo test` runs fully offline. The two tests that touch the network are
  `#[ignore]`d: `cargo test -- --ignored --nocapture` prints live percentages
  (`live_fetch`) and the raw rate-limit subtrees (`live_shape`), which is where to
  start if the endpoint's shape ever changes.
