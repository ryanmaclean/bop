# Web UI Dashboard

**Phase**: phase-4  **Priority**: could  **Complexity**: high  **Impact**: medium

## Description
A browser-based dashboard (served by `jc serve --ui`) for visualizing job state, browsing job outputs, managing providers, and viewing audit logs. Built as a lightweight single-page app (likely WASM or simple HTML+JS). Primarily for non-terminal users and remote monitoring.

## Rationale
A web UI broadens JobCard's appeal to developers who prefer graphical interfaces and enables remote monitoring without SSH. It also serves as the foundation for future team collaboration features.

## User Stories
- As a developer, I want a web UI so that I can monitor running agents from my phone while away from my desk
- As a team lead, I want to share a dashboard URL with non-technical stakeholders to show AI coding progress

## Acceptance Criteria
- `jc serve --ui` serves a web dashboard at http://localhost:8080/ui
- Dashboard shows live job status with auto-refresh via WebSocket or SSE
- Clicking a job shows its full details: spec, logs, output, audit trail
- Providers page shows all configured providers with status and cooldowns
- UI is functional without JavaScript enabled (server-side rendered fallback)