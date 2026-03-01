# JobCard: Heterogeneous Agent Orchestrator

A pluggable job system for parallel AI coding agents that uses APFS/Btrfs filesystem primitives as the control plane. Jobs are macOS directory bundles (UTI-registered "cards") navigable in Finder with Quick Look previews. The filesystem IS the state machine, the job queue, and the audit log.

## 🚀 Quick Start

```bash
# Build the project
cargo build

# Initialize the job card system
./target/debug/jc init

# Create a new job from a template
./target/debug/jc new implement my-feature

# Run the dispatcher (processes pending jobs)
./target/debug/jc dispatcher --max-workers 3

# Run the merge gate (merges completed jobs)
./target/debug/jc merge-gate

# Check status
./target/debug/jc status
```

## 🏗️ Architecture

```
┌─────────────────────────────────────────────┐
│  Finder / Quick Look / Spotlight            │  ← FREE (macOS native)
│  Vibe Kanban (optional web UI, Apache 2.0)  │  ← DON'T BUILD (fork if needed)
└───────────────────┬─────────────────────────┘
                    │ reads bundle state
┌───────────────────▼─────────────────────────┐
│  dispatcher (~250 LOC shell or Rust)        │  ← BUILD
│  - FSEvents/inotifywait watcher             │
│  - stage router                             │
│  - provider failover                        │
│  - dead worker reaper (launchd KeepAlive)   │
└───────────────────┬─────────────────────────┘
                    │ fork per job
┌───────────────────▼─────────────────────────┐
│  adapters/ (~20 LOC each)                   │  ← BUILD
│  claude / codex / goose / aider / ollama    │
└───────────────────┬─────────────────────────┘
                    │ works inside
┌───────────────────▼─────────────────────────┐
│  .jobcard bundles (APFS clones)             │  ← BUILD (structure + UTI)
│  each with git worktree                     │
└─────────────────────────────────────────────┘
```

## 📁 Directory Structure

```
.cards/
├── templates/           # Job card templates
│   ├── implement.jobcard/
│   ├── refactor.jobcard/
│   └── qa-only.jobcard/
├── pending/            # Jobs waiting to be processed
├── running/            # Currently executing jobs
├── done/               # Completed jobs awaiting merge
├── merged/             # Successfully merged jobs
├── failed/             # Failed jobs
└── providers.json      # Provider configuration
```

## 🎯 Design Principles

1. **The filesystem is the database.** No SQLite, no JSONL parsing, no in-process state. Directory position = job status. `mv` = state transition. `ls` = dashboard.
2. **Cards are bundles.** Each job is a macOS document bundle with a registered UTI. Finder, Spotlight, Quick Look treat it as a first-class object.
3. **Agents are adapters.** Three functions per agent runtime: `exec`, `is_alive`, `resume`. ~20 LOC each. Claude, Codex, Goose, Aider, OpenCode, local Ollama.
4. **Stages route to agents.** A single job flows through spec→plan→implement→QA. Each stage can target a different agent runtime and model tier.
5. **COW clones for templates.** APFS `cp -c` / Btrfs reflink. Creating 50 jobs from a template costs zero disk.
6. **Crash recovery is `mv failed/ pending/`.** No log replay. No state inference. Atomic rename.
7. **No GC languages in the critical path.** Dispatcher in Rust or shell. Adapters in shell. Quick Look plugin in Swift.

## 🔧 Installation

### Prerequisites

- macOS 12+ (for APFS COW clones and Quick Look integration)
- Rust 1.70+ (for building the CLI)
- Git (for worktree management)
- Optional: Ollama (for local model support)

### Build

```bash
# Clone the repository
git clone <repository-url>
cd gtfs

# Build the CLI tool
cargo build

# Install to system (optional)
sudo cp target/debug/jc /usr/local/bin/jc
```

### macOS Integration

```bash
# Install Quick Look plugin
xcodebuild -project JobCardQuickLook/JobCardQuickLook.xcodeproj -scheme JobCardQuickLook

# Register UTI (one-time)
open JobCardType/JobCardType.app

# Install launchd services (optional)
cp launchd/*.plist ~/Library/LaunchAgents/
launchctl load ~/Library/LaunchAgents/com.yourorg.jobcard.dispatcher.plist
launchctl load ~/Library/LaunchAgents/com.yourorg.jobcard.merge-gate.plist
```

## 📋 CLI Commands

```bash
jc init                          # Create .cards/ structure in current repo
jc new <template> <id>           # Clone template → pending/
jc new implement feat-auth       # Example
jc status                        # ls across all state directories
jc status feat-auth              # Show meta.json for specific card
jc validate <id>                 # Validate card structure
jc dispatcher                    # Run dispatcher daemon
jc merge-gate                    # Run merge gate daemon
jc retry <id>                    # Move card back to pending/
jc kill <id>                     # SIGTERM running card and move to failed/
jc logs <id> --follow            # Stream stdout/stderr for a card
jc inspect <id>                  # Show meta/spec/log summary
jc serve --port 8080             # Start REST API (default bind 127.0.0.1)
jc serve --bind 0.0.0.0 --port 8080
# WARNING: non-localhost --bind exposes unauthenticated job control endpoints.

# Model Management (v0.2.0+)
jc models list                   # List available models
jc models show <model>           # Show model details
jc models test <model>           # Test model capabilities
jc models compare <m1> <m2>      # Compare models
jc health check                  # Check provider health
jc costs report                  # Show cost breakdown
jc analytics performance         # Performance analytics
```

REST API endpoints served by `jc serve`:
- `GET /jobs`
- `GET /jobs/:id`
- `POST /jobs`
- `POST /jobs/:id/retry`
- `DELETE /jobs/:id`
- `GET /jobs/:id/logs` (Server-Sent Events)
- `GET /openapi.json`

## 🤖 Supported AI Providers

### Cloud Providers
- **Claude** (`adapters/claude.sh`) - Anthropic Claude API
- **Codex** (`adapters/codex.sh`) - OpenAI Codex API
- **Goose** (`adapters/goose.sh`) - Goose AI
- **Aider** (`adapters/aider.sh`) - Aider coding assistant
- **OpenCode** (`adapters/opencode.sh`) - OpenCode platform

### Local Providers
- **Ollama** (`adapters/ollama-local.sh`) - Local Ollama models
- **Mock** (`adapters/mock.sh`) - Mock adapter for testing

### Adding New Providers

Create a new shell script in `adapters/` following this pattern:

```bash
#!/usr/bin/env bash
set -euo pipefail

workdir="$1"; prompt_file="$2"; stdout_log="$3"; stderr_log="$4"

cd "$workdir" || exit 1

# Your agent command here
your-agent -p "$(cat "$prompt_file")" \
  > "$stdout_log" 2> "$stderr_log"
rc=$?

# Detect rate limits (exit code 75 = EX_TEMPFAIL)
if grep -qiE 'rate limit|429|too many requests' "$stderr_log"; then
  exit 75
fi

exit $rc
```

## 🔄 Job Lifecycle

1. **Create**: `jc new implement my-feature` clones template to `pending/`
2. **Dispatch**: Dispatcher moves job to `running/` and executes adapter
3. **Execute**: Agent processes the job in its worktree
4. **Complete**: Successful jobs move to `done/`
5. **Merge**: Merge gate validates acceptance criteria and merges to main
6. **Archive**: Merged jobs move to `merged/`

## 📊 Job Card Structure

```
my-feature.jobcard/                    ← macOS document bundle
├── Info.plist                        ← UTI declaration, bundle metadata
├── meta.json                        ← machine-readable job state
├── spec.md                           ← what to build (human-authored)
├── prompt.md                         ← agent prompt template with {{variables}}
├── QuickLook/
│   ├── Preview.html                  ← rendered card for Finder preview
│   └── Thumbnail.png                 ← 512x512 card thumbnail
├── worktree/                         ← git worktree checkout (created by dispatcher)
├── logs/
│   ├── stdout.log                    ← agent stdout
│   ├── stderr.log
│   └── pid                           ← agent process ID
└── output/
    ├── diff.patch                    ← final work product
    └── qa_report.md                  ← QA findings
```

## 🎨 macOS Integration

### Quick Look Preview
Job cards appear as rich previews in Finder showing:
- Job name and current stage
- Progress indicator (spec ✓ → plan ✓ → implement ⟳ → qa ○)
- Agent type and provider
- Acceptance criteria
- Recent activity logs

### Spotlight Integration
Search job cards with:
```bash
mdfind "kMDItemKind == 'Agent Job Card' && com_yourorg_jobcard_stage == 'failed'"
```

### UTI Registration
`.jobcard` files are registered as `com.yourorg.jobcard` conforming to:
- `com.apple.package` (treated as bundles)
- `public.composite-content` (contain multiple files)

## 🔧 Configuration

### Provider Configuration (`.cards/providers.json`)

```json
{
  "providers": {
    "claude": {
      "command": "adapters/claude.sh",
      "rate_limit_exit": 75
    },
    "ollama-local": {
      "command": "adapters/ollama-local.sh", 
      "rate_limit_exit": 75
    }
  }
}
```

### Template Configuration
Templates define job structure and default settings:
- Provider chain (which agents to try in order)
- Default stage (spec, plan, implement, qa)
- Acceptance criteria templates
- Prompt templates with variable substitution

### Model Lookup System

JobCard includes intelligent model selection based on task complexity, required capabilities, and cost constraints. Models are configured in `.cards/models.json`. See [MODEL_LOOKUP.md](MODEL_LOOKUP.md) for the full design.

## 🚦 Process Supervision

### Launchd Services (macOS)

```bash
# Start services
launchctl start com.yourorg.jobcard.dispatcher
launchctl start com.yourorg.jobcard.merge-gate

# Check status
launchctl list | grep jobcard

# View logs
tail -f /tmp/jobcard-dispatcher.log
tail -f /tmp/jobcard-merge-gate.log
```

### Systemd Services (Linux)

```bash
# Install services
sudo cp systemd/*.service /etc/systemd/system/
sudo systemctl enable jobcard-dispatcher
sudo systemctl enable jobcard-merge-gate

# Start services
sudo systemctl start jobcard-dispatcher
sudo systemctl start jobcard-merge-gate
```

## 🐛 Troubleshooting

### Common Issues

1. **Jobs stuck in `running/`**: Check if agent process is alive
   ```bash
   ps aux | grep -i agent
   # Or use reaper: dispatcher will automatically move dead jobs back to pending/
   ```

2. **Rate limiting**: Check provider cooldowns
   ```bash
   jc providers  # Shows provider status and cooldowns
   ```

3. **Merge conflicts**: Jobs fail during merge gate
   ```bash
   # Check conflict reports
   cat .cards/failed/job-name/output/conflicts.diff
   ```

4. **Missing templates**: Ensure templates directory exists
   ```bash
   jc init  # Creates default templates
   ```

### Debug Mode

```bash
# Run dispatcher with verbose output
RUST_LOG=debug ./target/debug/jc dispatcher --once

# Test individual adapter
bash adapters/claude.sh /path/to/workdir /path/to/prompt /tmp/stdout /tmp/stderr
```

## 📚 Documentation

| File | Description |
|------|-------------|
| [ROADMAP.md](ROADMAP.md) | Version milestones and planned features |
| [MODEL_LOOKUP.md](MODEL_LOOKUP.md) | Model selection system design |
| [PORTABLE_ADAPTERS.md](PORTABLE_ADAPTERS.md) | Cross-platform adapter guide |
| [IDEATION.md](IDEATION.md) | System vision and philosophy |

## 🤝 Contributing

1. Fork the repository
2. Create a feature branch
3. Add your adapter or improvement
4. Test with the mock adapter first
5. Submit a pull request

### Development Setup

```bash
# Install development dependencies
cargo install cargo-watch

# Run with auto-reload
cargo watch -x 'run -- jc dispatcher --once'

# Run tests
cargo test
```

## 📄 License

MIT License - see LICENSE file for details.

## 🙏 Acknowledgments

Inspired by:
- Gas Town's crash-recovery-via-externalized-state
- Auto-Claude's spec→plan→implement→QA pipeline
- HyperCard's card-as-tangible-object metaphor
- Vibe Kanban's pluggable executor architecture

## 🔗 Related Projects

- [Gas Town](https://github.com/gas-town) - Multi-agent orchestration system
- [Auto-Claude](https://github.com/auto-claude) - Automated coding pipeline
- [Vibe Kanban](https://github.com/vibe-kanban) - Pluggable task executor
