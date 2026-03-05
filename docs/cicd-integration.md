# CI/CD Integration

bop integrates with GitHub Actions and GitLab CI via a lightweight REST API server. Jobs run on a self-hosted runner; your CI pipeline creates jobs and polls for completion.

## Architecture

```
CI Pipeline (GitHub / GitLab)
        │
        │  HTTP (REST API)
        ▼
Self-hosted runner
  ┌─────────────────────────┐
  │  bop serve               │  ← REST API server
  │  bop dispatcher          │  ← picks up jobs, runs agents
  │  .cards/ filesystem     │  ← state machine
  └─────────────────────────┘
```

## Starting the API Server

On your self-hosted runner, start the bop API server alongside the dispatcher:

```bash
# Start the REST API server (default: localhost:8765)
bop serve --bind 127.0.0.1 --port 8765 &

# Start the dispatcher
bop dispatcher --adapter adapters/claude.sh &
```

> **Security**: The API server has no authentication. Keep `--bind 127.0.0.1` (loopback-only) unless you add a reverse proxy with authentication in front.

## REST API Reference

All endpoints are on the base URL provided to `bop serve`.

### `POST /jobs`

Create a new job.

**Request body:**
```json
{
  "template": "implement",
  "id": "my-job-001",
  "spec": "Implement OAuth2 login...",
  "acceptance_criteria": ["Tests pass", "Linter clean"]
}
```

**Response `201 Created`:**
```json
{
  "job": { "id": "my-job-001", "state": "pending", ... },
  "meta": { ... },
  "spec": "Implement OAuth2 login..."
}
```

### `GET /jobs/{id}`

Get current status of a job.

**Response `200 OK`:**
```json
{
  "job": {
    "id": "my-job-001",
    "state": "done",
    "stage": "implement",
    "created_at": "2026-03-01T18:00:00Z",
    "started_at": "2026-03-01T18:00:05Z",
    "finished_at": "2026-03-01T18:12:30Z"
  },
  "meta": { ... },
  "spec": "..."
}
```

States: `pending` → `running` → `done` | `failed` | `merged`

### `GET /jobs/{id}/output`

Retrieve output files and logs for a completed job.

**Response `200 OK`:**
```json
{
  "id": "my-job-001",
  "state": "done",
  "files": {
    "qa_report.md": "# QA Report\n...",
    "logs/stdout.log": "...",
    "logs/stderr.log": "..."
  }
}
```

### `GET /jobs/{id}/logs`

Stream stdout/stderr as Server-Sent Events (SSE) while a job is running.

### `POST /jobs/{id}/retry`

Move a failed/done job back to `pending`.

### `DELETE /jobs/{id}`

Kill a running job (sends SIGTERM, moves to `failed`).

### `GET /openapi.json`

Generated OpenAPI 3.0 specification for all endpoints.

---

## GitHub Actions

Three reusable composite actions are provided in `.github/actions/`.

### `bop-create`

Creates a job and outputs its ID.

```yaml
- uses: ./.github/actions/bop-create
  id: create
  with:
    api-url: http://127.0.0.1:8765
    template: implement
    spec: "Implement feature X"
    acceptance-criteria: '["Tests pass","Linter clean"]'
# outputs: steps.create.outputs.job-id
```

### `bop-wait`

Polls until the job completes. Fails the CI step if the job fails.

```yaml
- uses: ./.github/actions/bop-wait
  with:
    api-url: http://127.0.0.1:8765
    job-id: ${{ steps.create.outputs.job-id }}
    timeout-minutes: 60
    poll-interval-seconds: 15
# outputs: steps.wait.outputs.state (done|failed|merged)
```

### `bop-output`

Fetches output files and uploads them as a workflow artifact.

```yaml
- uses: ./.github/actions/bop-output
  with:
    api-url: http://127.0.0.1:8765
    job-id: ${{ steps.create.outputs.job-id }}
    output-dir: ai-output
# Uploads artifact: bop-output-<job-id>
```

### Full example workflow

See `.github/workflows/ai-coding-job.yml` for a complete workflow that:

1. Triggers on the `ai-implement` PR label
2. Creates a job with the PR context as the spec
3. Waits up to 45 minutes for completion
4. Fetches output and comments the QA report on the PR

---

## GitLab CI

Include the template from `.gitlab/ci-templates/bop.yml` and extend the hidden jobs:

```yaml
include:
  - local: '.gitlab/ci-templates/bop.yml'

variables:
  BOP_API_URL: http://127.0.0.1:8765

# Option 1: Separate create / wait / output stages
create-job:
  stage: create
  extends: .bop-create
  variables:
    BOP_TEMPLATE: implement
    BOP_SPEC: "Implement feature X"
    BOP_ACCEPTANCE_JSON: '["Tests pass"]'

wait-for-job:
  stage: wait
  extends: .bop-wait
  needs: [create-job]

fetch-output:
  stage: output
  extends: .bop-output
  needs: [wait-for-job]

# Option 2: All-in-one job
ai-coding:
  stage: deploy
  extends: .bop-run
  variables:
    BOP_TEMPLATE: implement
    BOP_SPEC: "Implement feature X"
```

### CI/CD Variables

| Variable | Default | Description |
|---|---|---|
| `BOP_API_URL` | `http://127.0.0.1:8765` | bop API server URL |
| `BOP_POLL_INTERVAL` | `15` | Seconds between status polls |
| `BOP_TIMEOUT_MINUTES` | `60` | Maximum wait time |

---

## Self-hosted Runner Setup

### GitHub Actions

1. Register a self-hosted runner in your repo settings
2. Install the `bop` binary on the runner
3. Create a launchd/systemd service for `bop serve` and `bop dispatcher`
4. Set `runs-on: self-hosted` in your workflow

### GitLab CI

1. Register a GitLab Runner with the `shell` executor on your machine
2. Ensure `bop`, `jq`, and `curl` are available in the runner's PATH
3. Start `bop serve` and `bop dispatcher` as background services
4. Set `BOP_API_URL` in your GitLab CI/CD variables
