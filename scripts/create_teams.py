#!/usr/bin/env python3
"""
Creates 5 team card directories from roadmap.json, 5 cards each.
Each team gets its own .cards/team-N/ dir + providers.json + dispatcher.
"""

import json, os, shutil, subprocess, pathlib, datetime

ROOT = pathlib.Path("/Users/studio/gtfs")
ROADMAP = ROOT / ".auto-claude/roadmap/roadmap.json"
JC = ROOT / "target/debug/jc"
ADAPTERS = ROOT / "adapters"

roadmap = json.loads(ROADMAP.read_text())
features = {f["id"]: f for f in roadmap["features"]}

# ── 5 teams × 5 cards ────────────────────────────────────────────────────────
# feature-1 (job-control) is split into retry/kill/logs to reach 25 total.

TEAMS = [
    {
        "name": "team-cli",
        "adapter": "claude",
        "desc": "CLI Commands & UX",
        "cards": [
            # feature-1 split into 3
            ("job-control-retry",  "feature-1", "Implement `jc retry <id>` command: move a failed card back to pending/, increment retry_count in meta.json, reset failure_reason."),
            ("job-control-kill",   "feature-1", "Implement `jc kill <id>` command: read PID from logs/pid (and xattr), send SIGTERM, move running card to failed/."),
            ("job-control-logs",   "feature-1", "Implement `jc logs <id>` command: stream stdout.log and stderr.log for a running or completed card."),
            ("shell-completions",  "feature-6", None),
            ("clean-command",      "feature-7", None),
        ],
    },
    {
        "name": "team-arch",
        "adapter": "claude",
        "desc": "Core Architecture",
        "cards": [
            ("config-file",               "feature-3",  None),
            ("cli-refactoring",           "feature-4",  None),
            ("providers-cli",             "feature-8",  None),
            ("worktree-cli",              "feature-9",  None),
            ("provider-schema-validation","feature-12", None),
        ],
    },
    {
        "name": "team-quality",
        "adapter": "aider",
        "desc": "Quality & Observability",
        "cards": [
            ("test-coverage",       "feature-5",  None),
            ("colored-output",      "feature-2",  None),
            ("audit-logs",          "feature-14", None),
            ("realtime-integration","feature-11", None),
            ("cicd-integration",    "feature-19", None),
        ],
    },
    {
        "name": "team-intelligence",
        "adapter": "opencode",
        "desc": "Intelligence Layer",
        "cards": [
            ("model-lookup-registry",   "feature-10", "Implement the model registry: read a .cards/models.json file defining available models with capabilities, pricing, and limits. Add `jc models list` CLI command."),
            ("model-lookup-capability", "feature-10", "Implement capability matching: given a job's stage and spec, score available models by proficiency. Store result as selected_model in meta.json before dispatch."),
            ("model-lookup-cost",       "feature-10", "Implement cost tracking: after each run, record tokens_used and cost_usd in meta.json stages record. Add `jc costs report` command."),
            ("fsevents-watcher",        "feature-13", None),
            ("cross-job-context",       "feature-17", None),
        ],
    },
    {
        "name": "team-platform",
        "adapter": "codex",
        "desc": "Platform & Growth",
        "cards": [
            ("tui-dashboard",     "feature-15", None),
            ("rest-api",          "feature-16", None),
            ("persistent-memory", "feature-18", None),
            ("web-ui",            "feature-20", None),
            ("ai-job-generation", "feature-22", None),
        ],
    },
]

def build_spec(feature_id, override_desc=None):
    f = features[feature_id]
    lines = [
        f"# {f['title']}",
        "",
        f"**Phase**: {f['phase_id']}  **Priority**: {f['priority']}  **Complexity**: {f['complexity']}  **Impact**: {f['impact']}",
        "",
        "## Description",
        override_desc if override_desc else f['description'],
        "",
        "## Rationale",
        f['rationale'],
        "",
        "## User Stories",
    ]
    for s in f.get("user_stories", []):
        lines.append(f"- {s}")
    lines += ["", "## Acceptance Criteria"]
    for c in f.get("acceptance_criteria", []):
        lines.append(f"- {c}")
    return "\n".join(lines)

def make_meta(card_id, feature_id, provider_name):
    f = features[feature_id]
    return {
        "id": card_id,
        "created": datetime.datetime.utcnow().isoformat() + "Z",
        "stage": "implement",
        "priority": {"must": 1, "should": 2, "could": 3, "wont": 4}.get(f["priority"], 3),
        "provider_chain": [provider_name, "mock"],
        "acceptance_criteria": f.get("acceptance_criteria", []),
        "worktree_branch": f"job/{card_id}",
        "retry_count": 0,
        "failure_reason": None,
        "stages": {},
        "agent_type": provider_name,
    }

def make_providers_json(adapter_name):
    adapter_path = str(ADAPTERS / f"{adapter_name}.zsh")
    return {
        "providers": {
            adapter_name: {
                "command": adapter_path,
                "rate_limit_exit": 75,
            },
            "mock": {
                "command": str(ADAPTERS / "mock.zsh"),
                "rate_limit_exit": 75,
            },
        }
    }

for team in TEAMS:
    cards_dir = ROOT / f".cards/{team['name']}"
    cards_dir.mkdir(parents=True, exist_ok=True)

    # Init directory structure
    for sub in ["templates", "pending", "running", "done", "merged", "failed"]:
        (cards_dir / sub).mkdir(exist_ok=True)

    # Write providers.json
    (cards_dir / "providers.json").write_text(
        json.dumps(make_providers_json(team["adapter"]), indent=2)
    )

    print(f"\n── {team['name']} ({team['adapter']}) ── {team['desc']}")

    for card_id, feature_id, override_desc in team["cards"]:
        card_dir = cards_dir / "pending" / f"{card_id}.jobcard"
        card_dir.mkdir(parents=True, exist_ok=True)
        (card_dir / "logs").mkdir(exist_ok=True)
        (card_dir / "output").mkdir(exist_ok=True)

        (card_dir / "spec.md").write_text(build_spec(feature_id, override_desc))
        (card_dir / "prompt.md").write_text(
            "You are working inside a Rust Cargo workspace (jobcard-core + jc crates).\n"
            "The project root is your working directory.\n\n"
            "{{spec}}\n\n"
            "Acceptance criteria:\n{{acceptance_criteria}}\n"
        )
        meta = make_meta(card_id, feature_id, team["adapter"])
        (card_dir / "meta.json").write_text(json.dumps(meta, indent=2))

        print(f"  ✓ {card_id} [{feature_id}]")

print("\n\nDone. Pending counts:")
for team in TEAMS:
    cards_dir = ROOT / f".cards/{team['name']}"
    n = len(list((cards_dir / "pending").iterdir()))
    print(f"  {team['name']}: {n} cards")
