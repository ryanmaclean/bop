#!/usr/bin/env python3
"""Lane throughput metrics implementation. Called by lane_metrics.nu."""
import json
import pathlib
import statistics
import sys
import time
from datetime import datetime, timezone

root = pathlib.Path(sys.argv[1])
window_minutes = int(sys.argv[2])
output = sys.argv[3]
cutoff = time.time() - (window_minutes * 60)

lanes = {
    "blue": root / ".cards-blue",
    "green": root / ".cards-green",
}


def parse_created(raw: str):
    if not raw:
        return None
    try:
        # Support chrono-style RFC3339 and Z suffix.
        return datetime.fromisoformat(raw.replace("Z", "+00:00")).timestamp()
    except Exception:
        return None


results = {}
for lane, lane_root in lanes.items():
    merged = 0
    failed = 0
    policy_violations = 0
    retries = 0
    orphan_recovery_events = 0
    lead_times = []

    for state in ("merged", "failed"):
        state_dir = lane_root / state
        if not state_dir.exists():
            continue

        for card in state_dir.glob("*.bop"):
            try:
                mtime = card.stat().st_mtime
            except OSError:
                continue
            if mtime < cutoff:
                continue

            meta = {}
            meta_path = card / "meta.json"
            try:
                with meta_path.open("r", encoding="utf-8") as fh:
                    meta = json.load(fh)
            except Exception:
                meta = {}

            if state == "merged":
                merged += 1
            else:
                failed += 1

            if meta.get("failure_reason") == "policy_violation":
                policy_violations += 1

            retry_count = int(meta.get("retry_count") or 0)
            if retry_count > 0:
                retries += 1
                orphan_recovery_events += retry_count

            created_ts = parse_created(meta.get("created", ""))
            if created_ts is not None and mtime >= created_ts:
                lead_times.append((mtime - created_ts) / 60.0)

    total = merged + failed
    success_rate = (merged / total) if total else 0.0
    policy_violation_rate = (policy_violations / total) if total else 0.0
    retry_rate = (retries / total) if total else 0.0
    lead_time_avg_min = statistics.fmean(lead_times) if lead_times else 0.0

    results[lane] = {
        "total_completed": total,
        "merged": merged,
        "failed": failed,
        "policy_violations": policy_violations,
        "policy_violation_rate": policy_violation_rate,
        "success_rate": success_rate,
        "retry_rate": retry_rate,
        "orphan_recovery_events": orphan_recovery_events,
        "lead_time_avg_min": lead_time_avg_min,
        "window_minutes": window_minutes,
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
    }

if output == "json":
    print(json.dumps(results, indent=2, sort_keys=True))
    sys.exit(0)

header = (
    "lane",
    "completed",
    "success_rate",
    "policy_violation_rate",
    "retry_rate",
    "lead_time_avg_min",
    "orphan_recovery_events",
)
print("\t".join(header))
for lane in ("blue", "green"):
    r = results[lane]
    row = (
        lane,
        str(r["total_completed"]),
        f"{r['success_rate']:.3f}",
        f"{r['policy_violation_rate']:.3f}",
        f"{r['retry_rate']:.3f}",
        f"{r['lead_time_avg_min']:.1f}",
        str(r["orphan_recovery_events"]),
    )
    print("\t".join(row))
