# Claude Usage

A fixed 800×480 desk panel that answers one question at a glance: **how much of the
5-hour window is gone, and how fast am I burning it?**

The frontend is complete and runs on mock telemetry. Nothing is wired to a real
data source yet.

## Run it

```bash
npm install
npm run dev          # frontend only, http://localhost:1420
npm run tauri dev    # the panel itself
```

## The design

A rolling window *is* a strip-chart recorder — time feeds right to left, the pen
sits at NOW, and what falls off the left edge is capacity coming back to you. So
the hero isn't a number in a card, it's the paper.

- **Oxide red belongs to the scale.** The grid, the limit line, and the over-range
  band are red; a model never is. If something on the paper is red, it is telling
  you about the ceiling.
- **Models are ink pens.** Opus blue, Sonnet green, Haiku graphite, stacked with
  graphite at the baseline so it never fuses with the graphite total pen riding
  the top. Each channel signs its own boundary at full ink.
- **The total is its own pen and always writes.** Clicking a channel in the legend
  lifts its wash but leaves the total line — so the gap you open up *is* that
  model's contribution.
- **The recorder annotates its own high-water mark.** The peak sits on the paper
  between the trace and the limit line, where a real instrument would print it.
- Type is Archivo at width 125 for readouts (stamped, not default-grotesque) and
  Martian Mono for silkscreen labels.

One motion moment: on load the paper feeds through left to right and the readouts
count up. `prefers-reduced-motion` turns all of it off.

## Wiring it up

Everything downstream reads `snapshot` from [data.js](src/data.js) — replace that
module with a Tauri command and the panel follows. The shape it expects:

| field | meaning |
| --- | --- |
| `series[]` | one entry per model: `values` in percent of the 5-hour limit, 60 buckets of 5 minutes |
| `used` | current total, percent of limit |
| `burnPerMin` | tokens per minute |
| `minutesToLimit` | projection at the current rate |
| `resetsInSeconds` | countdown for the title rail |
| `weekly` | `{ pct, segments, resetsLabel }` for the footer |

## Notes

- Window chrome is drawn by the frontend (`decorations: false`), so the title rail
  is the drag region and carries its own minimize/close.
- Window controls use `withGlobalTauri` rather than `@tauri-apps/api`, so there is
  no JS dependency on Tauri; the panel still runs in a plain browser.
- Fonts come from Google Fonts. Vendor them into `src/` before shipping so the
  panel renders offline.
