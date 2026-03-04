#!/usr/bin/env nu
# Policy check script — validates staged changes or card-scoped diffs
# against configurable rules (file caps, scope, decision records, etc.).
#
# The heavy lifting is done by an embedded Python3 script; this Nushell
# wrapper sets up environment variables and interprets the JSON result.

def main [
  --mode: string = "staged"      # staged | card
  --cards-dir: string = ".cards" # Cards root directory
  --card-dir: string = ""        # Required for --mode card
  --id: string = ""              # Optional card id hint
  --repo-root: string = ""       # Override git root
  --staged                       # Shortcut for --mode staged
  --json                         # Print JSON result
  --help (-h)                    # Show usage
] {
  if $help {
    print "Usage:"
    print "  scripts/policy_check.nu --staged [--cards-dir .cards]"
    print "  scripts/policy_check.nu --mode card --card-dir <path> [--cards-dir .cards]"
    print ""
    print "Options:"
    print "  --mode staged|card"
    print "  --staged                  Shortcut for --mode staged"
    print "  --card-dir <path>         Required for --mode card"
    print "  --id <id>                 Optional card id hint"
    print "  --cards-dir <path>        Cards root (default .cards)"
    print "  --repo-root <path>        Override git root"
    print "  --json                    Print JSON result"
    exit 0
  }

  # Resolve effective mode — --staged flag overrides --mode
  mut effective_mode = $mode
  if $staged {
    $effective_mode = "staged"
  }

  if $effective_mode != "staged" and $effective_mode != "card" {
    print -e $"Invalid --mode: ($effective_mode)"
    exit 2
  }

  if $effective_mode == "card" and $card_dir == "" {
    print -e "--card-dir is required for --mode card"
    exit 2
  }

  # Resolve repo root
  mut resolved_root = $repo_root
  if $resolved_root == "" {
    let git_result = do { ^git rev-parse --show-toplevel } | complete
    if $git_result.exit_code == 0 {
      $resolved_root = ($git_result.stdout | str trim)
    } else {
      $resolved_root = (pwd)
    }
  }

  # Export environment for the Python script
  $env.POLICY_MODE = $effective_mode
  $env.POLICY_CARDS_DIR = $cards_dir
  $env.POLICY_CARD_DIR = $card_dir
  $env.POLICY_CARD_ID = $id
  $env.POLICY_REPO_ROOT = $resolved_root

  # Embedded Python policy engine — kept as-is from the ZSH version
  let python_script = r#'
import json
import os
import pathlib
import re
import subprocess
import sys

try:
    import tomllib
except Exception:
    tomllib = None

mode = os.environ.get("POLICY_MODE", "staged")
cards_dir = pathlib.Path(os.environ.get("POLICY_CARDS_DIR", ".cards"))
card_dir_env = os.environ.get("POLICY_CARD_DIR", "")
card_id = os.environ.get("POLICY_CARD_ID", "")
repo_root = pathlib.Path(os.environ.get("POLICY_REPO_ROOT", ".")).resolve()

policy = {
    "allow_new_top_level_files": ["README.md", "Cargo.lock", "deny.toml"],
    "max_changed_files": 120,
    "max_changed_loc": 4000,
    "nontrivial_changed_files": 12,
    "nontrivial_changed_loc": 600,
    "decision_required_if_cli_change": True,
    "require_decision_for_nontrivial": True,
    "enforce_scope_without_policy_scope": False,
    "skip_when_no_git": True,
}

policy_path = cards_dir / "policy.toml"
if policy_path.exists() and tomllib is not None:
    with policy_path.open("rb") as f:
        loaded = tomllib.load(f)
    if isinstance(loaded, dict):
        policy.update(loaded)

RUNTIME_STATES = ("pending", "running", "done", "failed", "merged")


def run(cmd, cwd=None):
    return subprocess.run(cmd, cwd=cwd, text=True, capture_output=True)


def in_git_repo(path: pathlib.Path) -> bool:
    return run(["git", "-C", str(path), "rev-parse", "--is-inside-work-tree"]).returncode == 0


def parse_name_status(raw: str):
    out = []
    for line in raw.splitlines():
        line = line.strip()
        if not line:
            continue
        parts = line.split("\t", 1)
        if len(parts) != 2:
            continue
        out.append((parts[0], parts[1]))
    return out


def parse_numstat(raw: str):
    rows = []
    for line in raw.splitlines():
        line = line.strip()
        if not line:
            continue
        parts = line.split("\t")
        if len(parts) != 3:
            continue
        a, d, p = parts
        try:
            ai = int(a) if a.isdigit() else 0
            di = int(d) if d.isdigit() else 0
        except Exception:
            ai = di = 0
        rows.append((ai, di, p))
    return rows


def normalize_rel(path: str) -> str:
    p = path.replace("\\", "/").lstrip("./")
    while "//" in p:
        p = p.replace("//", "/")
    return p.rstrip("/")


cards_rel = None
try:
    cards_rel = normalize_rel(str(cards_dir.resolve().relative_to(repo_root)))
except Exception:
    if not cards_dir.is_absolute():
        cards_rel = normalize_rel(str(cards_dir))


def runtime_roots():
    roots = [f".cards/{s}" for s in RUNTIME_STATES]
    if cards_rel:
        roots.extend(f"{cards_rel}/{s}" for s in RUNTIME_STATES)
    uniq = []
    for r in roots:
        if r not in uniq:
            uniq.append(r)
    return uniq


if mode == "staged":
    git_ctx = repo_root
    diff_base_cmd = ["git", "-C", str(git_ctx), "diff", "--cached"]
else:
    card_dir = pathlib.Path(card_dir_env).resolve()
    if not card_dir.exists():
        print(json.dumps({"ok": False, "reasons": [f"card dir not found: {card_dir}"]}))
        raise SystemExit(0)
    ws = card_dir / "workspace"
    git_ctx = ws if ws.exists() else card_dir
    diff_base_cmd = ["git", "-C", str(git_ctx), "diff"]

if not in_git_repo(git_ctx):
    if policy.get("skip_when_no_git", True):
        print(json.dumps({
            "ok": True,
            "reasons": [f"skipped policy check: no git repo at {git_ctx}"],
            "metrics": {"changed_files": 0, "changed_loc": 0},
            "skipped": True,
        }))
        raise SystemExit(0)
    print(json.dumps({"ok": False, "reasons": [f"not a git repo: {git_ctx}"]}))
    raise SystemExit(0)

name_status_raw = run(diff_base_cmd + ["--name-status", "--no-renames"], cwd=git_ctx).stdout
numstat_raw = run(diff_base_cmd + ["--numstat", "--no-renames"], cwd=git_ctx).stdout
ns = parse_name_status(name_status_raw)
numstat = parse_numstat(numstat_raw)

changed_paths = []
for _, p in ns:
    changed_paths.append(p)

changed_file_count = len(set(changed_paths))
changed_loc = sum(a + d for a, d, _ in numstat)

reasons = []

# Runtime card directories should not be committed as source changes.
for p in sorted(set(changed_paths)):
    p_norm = normalize_rel(p)
    for root in runtime_roots():
        if not p_norm.startswith(root + "/"):
            continue
        if ".bop/" not in p_norm:
            continue
        if p_norm.endswith("/meta.json"):
            reasons.append(f"runtime card meta changed in VCS diff: {p}")
        else:
            reasons.append(f"runtime card path changed in VCS diff: {p}")
        break

# Rule 1: top-level file allowlist for additions.
allow_top = set(policy.get("allow_new_top_level_files", []))
for status, path in ns:
    if status != "A":
        continue
    if "/" in path:
        continue
    if path not in allow_top:
        reasons.append(f"new top-level file not allowed: {path}")

# Rule 4: size caps.
max_files = int(policy.get("max_changed_files", 120))
max_loc = int(policy.get("max_changed_loc", 4000))
if changed_file_count > max_files:
    reasons.append(f"changed files {changed_file_count} exceed max {max_files}")
if changed_loc > max_loc:
    reasons.append(f"changed LOC {changed_loc} exceed max {max_loc}")

# Rule 4b: card-copy/card-compression paths must keep APFS/reflink semantics.
copy_guard_files = {
    "scripts/route_canary.nu",
    "scripts/ingest_roadmap_hotfolder.nu",
    "scripts/macos_cards_maintenance.nu",
}
for p in sorted(set(changed_paths)):
    if normalize_rel(p) not in copy_guard_files:
        continue
    patch = run(diff_base_cmd + ["--", p], cwd=git_ctx).stdout
    for line in patch.splitlines():
        if not line.startswith("+") or line.startswith("+++"):
            continue
        added = line[1:].strip()
        compact = " ".join(added.split())
        if re.search(r"\bcp\s+-R\b|\bcp\s+-r\b", compact):
            if "-c" not in compact and "--reflink=auto" not in compact:
                reasons.append(
                    f"{p}: plain recursive cp added; require cp -c (macOS) or --reflink=auto"
                )
        if "ditto" in compact:
            allowed = (
                "--clone" in compact
                or "--hfsCompression" in compact
                or "--preserveHFSCompression" in compact
                or ("-c" in compact and "-k" in compact)
            )
            if not allowed:
                reasons.append(
                    f"{p}: ditto added without clone/compression flags"
                )

# Rule 2: CLI command churn requires decision record.
cli_change_requires_decision = bool(policy.get("decision_required_if_cli_change", True))
cli_subcommand_change = False
if any(p == "crates/jc/src/main.rs" or p.endswith("/crates/jc/src/main.rs") for p in changed_paths):
    patch = run(diff_base_cmd + ["--", "crates/jc/src/main.rs"], cwd=git_ctx).stdout
    for line in patch.splitlines():
        if not line.startswith("+") or line.startswith("+++"):
            continue
        if re.search(r"^\+\s*[A-Z][A-Za-z0-9_]*\s*\{", line):
            cli_subcommand_change = True
            break

# Card-only scope/decision checks.
meta = {}
decision_path = None
if mode == "card":
    card_dir = pathlib.Path(card_dir_env).resolve()
    meta_path = card_dir / "meta.json"
    for required in ("meta.json", "logs", "output"):
        target = card_dir / required
        if not target.exists():
            reasons.append(f"card bundle missing required path: {target}")

    if meta_path.exists():
        try:
            meta = json.loads(meta_path.read_text(encoding="utf-8"))
            if not isinstance(meta, dict):
                reasons.append(f"invalid meta.json object: {meta_path}")
                meta = {}
        except Exception:
            reasons.append(f"invalid meta.json: {meta_path}")

    wf = meta.get("workflow_mode")
    if wf is not None and (not isinstance(wf, str) or not wf.strip()):
        reasons.append("meta.workflow_mode must be a non-empty string when set")
    step = meta.get("step_index")
    if step is not None:
        if not isinstance(step, int) or step < 1:
            reasons.append("meta.step_index must be an integer >= 1 when set")
        if wf is None:
            reasons.append("meta.step_index requires meta.workflow_mode")

    # Rule 3: changed paths must stay in policy scope (if provided).
    scope_paths = meta.get("policy_scope") or []
    if isinstance(scope_paths, str):
        scope_paths = [scope_paths]
    scope_paths = [s.strip().rstrip("/") for s in scope_paths if isinstance(s, str) and s.strip()]

    if scope_paths:
        for p in set(changed_paths):
            if any(p == s or p.startswith(s + "/") for s in scope_paths):
                continue
            reasons.append(f"path outside policy_scope: {p}")
    elif bool(policy.get("enforce_scope_without_policy_scope", False)):
        reasons.append("policy_scope missing in meta.json while scope enforcement is enabled")

    # Rule 5: decision.md required for non-trivial work or explicit decision_required.
    explicit_decision_required = bool(meta.get("decision_required", False))
    nontrivial = (
        changed_file_count >= int(policy.get("nontrivial_changed_files", 12))
        or changed_loc >= int(policy.get("nontrivial_changed_loc", 600))
    )

    decision_cfg = meta.get("decision_path")
    if isinstance(decision_cfg, str) and decision_cfg.strip():
        p = pathlib.Path(decision_cfg)
        decision_path = p if p.is_absolute() else (card_dir / p)
    else:
        decision_path = card_dir / "decision.md"

    need_decision = explicit_decision_required
    if bool(policy.get("require_decision_for_nontrivial", True)) and nontrivial:
        need_decision = True
    if cli_change_requires_decision and cli_subcommand_change:
        need_decision = True

    if need_decision and not decision_path.exists():
        reasons.append(f"decision record missing: {decision_path}")

elif cli_change_requires_decision and cli_subcommand_change:
    reasons.append("CLI subcommand-like changes detected; use card mode with decision record")

ok = len(reasons) == 0
print(json.dumps({
    "ok": ok,
    "reasons": reasons,
    "metrics": {
        "changed_files": changed_file_count,
        "changed_loc": changed_loc,
        "cli_subcommand_change": cli_subcommand_change,
        "mode": mode,
        "card_id": card_id,
    },
}, ensure_ascii=False))
'#

  let result = (^python3 -c $python_script | str trim)

  if $json {
    print $result
  }

  let ok = ($result | from json | get ok)

  if $ok {
    print "POLICY PASS"
    exit 0
  }

  print "POLICY FAIL"
  let parsed = ($result | from json)
  let reasons = ($parsed | get reasons)
  for r in $reasons {
    print $"- ($r)"
  }
  let metrics = ($parsed | get metrics)
  print $"changed_files=($metrics | get changed_files) changed_loc=($metrics | get changed_loc)"
  exit 1
}
