# Spec 043 — bop gantt: HTML polish + card hover details

## Overview

`bop gantt --html` generates a static HTML Gantt timeline. The current output
is functional but bare. This spec adds:
- Card hover tooltips showing id, stage, provider, tokens, cost, duration
- Duration-based color heatmap (green=fast, yellow=medium, red=slow)
- A summary statistics row below the chart
- Responsive width (auto-fits to viewport without scrolling on standard screens)

## Features

### Hover tooltips

Each Gantt bar on hover shows a floating tooltip:
```
id: team-arch/spec-031
stage: implement
provider: codex
duration: 4m 22s
tokens: 12,400
cost: $0.18
```
Implemented with CSS `title` attribute or a JS `mouseover` event handler
(prefer CSS-only if possible for zero-JS fallback).

### Duration heatmap

Color-code bars by duration percentile within the visible window:
- p0–p50: `#4caf50` (green)
- p50–p80: `#ff9800` (amber)
- p80–p100: `#f44336` (red)

Compute percentiles client-side from the data embedded in the HTML or
server-side in the Rust renderer.

### Summary row

Below the chart, render a `<table>` with:
| Metric | Value |
|--------|-------|
| Total cards | N |
| Completed | N |
| Failed | N |
| Avg duration | Xm Ys |
| Total cost | $X.XX |
| Parallelism peak | N concurrent |

### Responsive layout

Set `max-width: 100%; overflow-x: auto` on the chart container.
The default `--html` output should look good at 1280px width without
horizontal scrolling.

## Acceptance Criteria

- [ ] `bop gantt --html` opens in browser without JS errors
- [ ] Hovering a bar shows the tooltip with card details
- [ ] Bars are color-coded by duration heatmap
- [ ] Summary stats table renders below chart
- [ ] Page is readable at 1280px width (no horizontal scroll on main chart)
- [ ] `cargo clippy -- -D warnings` clean
- [ ] `cargo test` passes

## Files to modify

- `crates/bop-cli/src/gantt.rs` — HTML template + data embedding
