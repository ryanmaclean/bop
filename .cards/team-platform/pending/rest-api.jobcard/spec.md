# REST API for External Integration

**Phase**: phase-3  **Priority**: could  **Complexity**: high  **Impact**: high

## Description
Add an optional HTTP server mode (`jc serve --port 8080`) exposing REST endpoints: GET /jobs (list), GET /jobs/:id (inspect), POST /jobs (create), POST /jobs/:id/retry, DELETE /jobs/:id (kill), GET /jobs/:id/logs (stream). Enables programmatic integration without the CLI binary.

## Rationale
A REST API unlocks integration with GitHub Actions, custom dashboards, monitoring tools, and team automation scripts. It's the bridge between JobCard as a local CLI and JobCard as a platform component in larger automation workflows.

## User Stories
- As a DevOps engineer, I want a REST API so that my GitHub Actions workflow can create jobs and poll for completion without shelling out to the jc binary
- As a developer, I want to query job state from my own dashboard scripts so that I can build custom monitoring on top of JobCard

## Acceptance Criteria
- `jc serve --port 8080` starts an HTTP server backed by the local .cards/ directory
- GET /jobs returns all jobs with state, provider, and timestamps
- POST /jobs accepts a JSON body with template, id, and spec to create a new job
- GET /jobs/:id/logs streams stdout/stderr as Server-Sent Events
- Server binds to localhost by default; --bind flag for remote access with documented security warning
- OpenAPI spec is generated and served at GET /openapi.json