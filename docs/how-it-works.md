# How it works

The details behind the panel: where each number comes from, why the chart is
drawn the way it is, and what the code is allowed to do. The
[README](../README.md) covers what it does and how to set it up.

## Where the numbers come from

Claude Code appends every exchange to JSONL under `~/.claude/projects/`. Each
assistant record carries the model, a timestamp, and the full token breakdown.
Two things about that data are easy to get wrong, and both are handled:

- **Half the records are duplicates.** Resuming or branching a session re-logs
  requests it already wrote: on this machine, 2,450 of 4,896. Events are
  deduplicated on `(requestId, message.id)`; skip that and every number roughly
  doubles.
- **Raw token counts are meaningless.** Cache reads run about 7× everything else
  combined, so an unweighted total is really just a cache-read counter. Tokens are
  weighted into input-equivalents at the published price ratios (output 5×, cache
  writes 1.25×/2×, cache reads 0.1×), because limits track compute, not volume.

Files are re-read incrementally: session logs only grow, so each poll parses just
the bytes appended since the last one.

## Blocks, not a rolling window

The 5-hour limit is not a rolling window. A block opens at a message sent outside
the previous block, runs five hours **from that instant**, then resets, which is
why `/usage` can name a reset time. So the chart shows **one block end to end**:
the pen sits at now, and the empty space to its right is the time you have left.
The dashed line is where the current burn rate lands you, and if it crosses the
ceiling the chart marks the clock time it happens.

Boundaries are **not** hour aligned (this machine's endpoint reports five-hour
resets at ten past), and while usage continues each block chains straight on from
the last, so the edge normally falls in a silence with no message to mark it.
That is why the reset instant is [remembered](#where-100-comes-from): it is the
one fact about the axis the local logs cannot yield.

## Where 100% comes from

Nothing on disk records your plan's ceiling. The endpoint reports the percentage
spent, so dividing our own token count by it solves for the limit, which means
**there is nothing to calibrate**. Three sources, in order of trust; the title rail
and footer always say which is in play.

**Live.** Claude Code renders `/usage` by calling `GET /api/oauth/usage` with the
OAuth token in `~/.claude/.credentials.json`; the panel reads the same endpoint.
True percentage, true reset instant, solved limit. The reported reset lands a
fraction either side of the real boundary (both `07:59:59.9` and `08:00:00.1`
show up for the same edge), so it is snapped to the nearest minute. Truncating
that to an hour for an axis label loses a whole hour.

**Remembered.** The solved limit **and both reset instants** are kept, so losing
the network drops the panel to the last known real limit rather than to a guess,
and leaves the time axis where live had it. While a remembered reset is still in
the future it pins the block outright; once it passes, the block chain restarts
from it rather than from the top of the log. Re-deriving the boundary from the
log alone had put the offline axis 2h10m from the live one, and swept in usage
the server had already reset away.

**Estimated.** Before any live reading has ever succeeded, it scales to the
busiest block on record. On this machine that guessed 17.1M against a real 6.31M,
a 2.7x error, so treat it as a shape, not a number. The title rail says so.

There is deliberately **no hand-entered fallback**. Reading a percentage out of
`/usage` needs the same token and the same network the endpoint itself needs, so
anything a person could type in, the app could have fetched. A calibration form
would only ever be a slower path to a number already in reach.

Constraints on the credential path:

- **Read only.** The credentials file is never written. Refresh tokens normally
  rotate, so refreshing here without persisting could invalidate the CLI's own
  login. An expired token is reported, not repaired: run any Claude Code command
  and it refreshes itself.
- **The token goes to one host** and is never logged, returned through a command,
  or put in an error message. A failed parse names only the windows we looked for.
- **Every failure is soft**, cached for a minute on success and two on failure,
  or for whatever `Retry-After` asks when throttled. The endpoint is undocumented:
  it can change or vanish, and the panel has to keep working from local logs when
  it does.
- **Failures are named for what they are.** Only a response we cannot parse reports
  that the endpoint changed; a 429 says it is being checked too often and a 5xx
  reports the status. Conflating those sends you hunting a break that is not there.
- `HTTPS_PROXY` and friends are honoured, so a proxied network does not become the
  one case where `/usage` works and the panel does not.

## Design

The usage chart is a chart recorder at heart: a pen at now, writing one block
from left to right. The rules that carry information:

- **Red belongs to the limit**: the limit line, over-range band, and the
  projected crossing. A model is never red. If something on the chart is red, it
  is telling you about the ceiling.
- **Each model is a coloured series.** Its wash fades to the baseline and its
  boundary glows, and the same colour marks its dot in the legend.
- **The total is its own white line and always draws.** Toggling a model lifts its
  wash but leaves the total line, so the gap you open is that model's contribution.
- **One model means no legend.** A single model's percentage *is* the hero number,
  so the legend collapses and the chart takes the space.

Every page shares one look, after ponytail's board: black ground, quiet cards, a
coloured dot naming each series, numbers set large. The panel is a 5-inch display
read from a few feet away, so no text is smaller than 14px. Type is Geist for
labels and Geist Mono for numbers. The theme button in the header cycles dark,
light, and system (follows Windows); light mode uses deeper shades of each colour
so lines and icons stay readable on white. One motion moment on load;
`prefers-reduced-motion` turns it all off.

## System page

It samples every 1.5 s:

- **CPU load, RAM, swap** from `sysinfo`.
- **CPU clock** from the Windows `% Processor Performance` counter, scaled from
  the base clock, so it follows boost. Elsewhere it is `sysinfo`'s figure.
- **GPU** (load, clock, power, temperature, VRAM, fan) from NVML, which ships
  with the NVIDIA driver. With no NVIDIA GPU the row hides.
- **CPU temperature.** Windows only exposes it to tools with a kernel driver, so
  the app reads it from [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor)
  when that is running, and never needs admin itself. Set it up once: run it
  (it asks for admin to load its driver), turn on Options → Remote Web Server
  → Run on the default port 8085, and Options → Run On Windows Startup. Without
  it the temperature line hides. On Linux `sysinfo` reads it directly.

## Layout

| | |
| --- | --- |
| [src/chart.js](../src/chart.js) | the usage chart: scale, series, projection |
| [src/main.js](../src/main.js) | polling, readouts, status |
| [src/api.js](../src/api.js) | IPC, with a synthetic fallback for browser dev |
| [src-tauri/src/usage.rs](../src-tauri/src/usage.rs) | log scanning, dedup, weighting |
| [src-tauri/src/blocks.rs](../src-tauri/src/blocks.rs) | block detection, snapshot |
| [src-tauri/src/oauth.rs](../src-tauri/src/oauth.rs) | live usage endpoint |
| [src-tauri/src/settings.rs](../src-tauri/src/settings.rs) | remembers the solved limit and the reset instants |
| [src/system.js](../src/system.js) | the System page: readouts and sparklines |
| [src-tauri/src/system.rs](../src-tauri/src/system.rs) | CPU, memory and GPU sampling, pushed as `stats` events |
| [src/macros.js](../src/macros.js) | the Macros page grid |
| [src-tauri/src/macros.rs](../src-tauri/src/macros.rs) | reads macros.json and runs a button's action |

## Notes

- The webview runs under a strict CSP: `script-src 'self'`, `object-src 'none'`,
  and network limited to the IPC channel; nothing loads from outside the app.
  `style-src` still allows `'unsafe-inline'` because Vite injects `<style>` blocks
  in dev and the markup sets a few `style` attributes. `freezePrototype` is on, and the app is
  granted no filesystem, shell, or HTTP plugin permissions, only window
  close/minimize/drag. Macros run on the Rust side, and only as macros.json
  describes them.
- Windows moves the cursor to every touch. A low-level mouse hook
  ([src-tauri/src/touch.rs](../src-tauri/src/touch.rs)) remembers where the real
  mouse last was, skipping the events Windows tags as made from touch, and puts
  the cursor back once every finger lifts. Using the mouse on the panel itself is
  unaffected.
- Window chrome is drawn by the frontend (`decorations: false`), so the title rail
  is the drag region and carries its own minimize/close.
- Window controls use `withGlobalTauri`, so there is no JS dependency on Tauri and
  the panel still runs in a plain browser.
- Geist and Geist Mono are bundled in [src/fonts](../src/fonts) with their license
  ([OFL.txt](../src/fonts/OFL.txt)), so the panel looks the same offline.
- Weekly usage comes from the endpoint's `seven_day` window, with its real reset
  time. Without live data it falls back to a rolling 7-day total labelled `7-day`
  and claims no reset schedule, because the logs do not record one.
- `cargo test` runs fully offline. The two tests that touch the network are
  `#[ignore]`d: `cargo test -- --ignored --nocapture` prints live percentages
  (`live_fetch`) and the raw rate-limit subtrees (`live_shape`), which is where to
  start if the endpoint's shape ever changes.
