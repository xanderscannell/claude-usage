# Changelog

All notable changes to Claude Usage are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0] - 2026-11-02

The first release.

### Usage page

- Charts the current 5-hour block from start to reset, stacked by model, with
  the total, where your burn rate lands you, and the time you'll hit the limit
  if you're on pace to.
- Reads Claude Code's session logs under `~/.claude/projects/`, only the new
  part on each poll. Drops the duplicate records that resumed sessions write and
  weights tokens by price, because limits track compute, not volume.
- Reads the true percentage and reset times from the endpoint Claude Code's
  `/usage` calls, with Claude Code's own login, read-only. Your plan's limit is
  solved from that, so there is nothing to calibrate.
- Remembers the last real limit and reset times, so offline it keeps the last
  known limit instead of guessing. The header always says which source is in
  play: Live, Remembered, or Estimated.
- Shows weekly usage in the footer, with its real reset time when live.
- Tap a model in the legend to lift it out of the stack.

### System page

- CPU and GPU load, clocks, temperatures, memory, VRAM, fan and power, sampled
  every 1.5 seconds, with a live graph for each.
- The CPU clock follows boost on Windows instead of showing the base speed.
- CPU temperature through LibreHardwareMonitor when it is running, so the panel
  never needs admin. GPU readouts through NVML on NVIDIA cards; without one, the
  GPU card hides.

### Macros page

- A Stream Deck style grid of keys, defined in `macros.json`: media keys,
  keyboard shortcuts, opening apps, files or URLs, and PowerShell commands.
- Saving the file updates the page within two seconds. A mistake shows on the
  key it belongs to, or in place of the grid if the file will not parse.
- The window never takes focus, so a shortcut lands in the app you were using.
- Keys are drawn as lit screens under glass caps.

### The panel

- A fixed 800×480 window for a 5-inch display. Swipe between pages, or drag
  with a mouse. A header button snaps the window onto the display it is sized
  for.
- Touching the panel no longer moves your mouse pointer to it.
- One design across all three pages, with no text smaller than 14px so it reads
  from a few feet away.
- Dark, light, and system themes from a button in the header. A new install
  starts on system, which follows Windows.
- Fonts are bundled, so the panel looks the same with the network off.

### Security

- A strict content security policy, and no plugin permissions beyond the window
  controls.
- The Claude Code credentials file is only ever read. The token goes to one host
  and is never logged or shown.
- The page names a macro by position and label only; what it runs is read from
  `macros.json` on the Rust side.

[1.0.0]: https://github.com/xanderscannell/claude-usage/releases/tag/v1.0.0
