# Terminal UI (TUI) Dashboard

**Phase**: phase-3  **Priority**: could  **Complexity**: high  **Impact**: high

## Description
Add a `jc dashboard` command that opens a ratatui-based TUI showing all jobs in real-time: a table view with job ID, state, provider, duration, and last log line; keyboard shortcuts for inspecting, retrying, and killing jobs; and auto-refresh via FSEvents/inotify.

## Rationale
Repeatedly running `jc status` to monitor 10+ concurrent jobs is tedious. A live TUI dashboard gives developers at-a-glance visibility into all running agents without leaving the terminal—a significant quality-of-life improvement for power users.

## User Stories
- As a developer running 10+ parallel agents, I want a live dashboard so that I can monitor all jobs at a glance and intervene when one fails
- As an AI engineer, I want keyboard shortcuts in the dashboard so that I can retry or kill jobs without switching to a separate terminal

## Acceptance Criteria
- `jc dashboard` opens a full-terminal TUI that auto-updates when job state changes
- Table shows: job ID, state (color-coded), provider, elapsed time, last log line
- Press 'r' on a selected job to retry, 'k' to kill, 'l' to open log view, 'q' to quit
- Log view shows streaming job stdout/stderr with scroll support
- TUI falls back gracefully to `jc status` output if terminal is too small or non-interactive