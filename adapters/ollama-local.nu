#!/usr/bin/env nu
# ollama-local.nu — run a card prompt against a local Ollama model
#
# Usage (called by dispatcher):
#   ollama-local.nu <workdir> <prompt_file> <stdout_log> <stderr_log> [timeout]
#
# Exit codes:
#   0   success
#   75  transient (ollama not running, model not loaded) → pending/, rotate provider
#   1   other failure → failed/
#
# Env vars:
#   OLLAMA_MODEL   model to use (default: qwen2.5-coder:7b)
#   OLLAMA_HOST    base URL (default: http://localhost:11434)

def main [
    workdir: string,
    prompt_file: string,
    stdout_log: string,
    stderr_log: string,
    _memory_out?: string,  # memory output file; read via JOBCARD_MEMORY_OUT env
] {
    let orig_dir = (pwd)
    let prompt_abs = if ($prompt_file | str starts-with "/") { $prompt_file } else { $"($orig_dir)/($prompt_file)" }
    let stdout_abs = if ($stdout_log | str starts-with "/") { $stdout_log } else { $"($orig_dir)/($stdout_log)" }
    let stderr_abs = if ($stderr_log | str starts-with "/") { $stderr_log } else { $"($orig_dir)/($stderr_log)" }

    cd $workdir

    let model = if "OLLAMA_MODEL" in $env { $env.OLLAMA_MODEL } else { "qwen2.5-coder:7b" }
    let host = if "OLLAMA_HOST" in $env { $env.OLLAMA_HOST } else { "http://localhost:11434" }

    # Health check
    let health = (do { ^curl -sf $"($host)/api/tags" } | complete)
    if $health.exit_code != 0 {
        $"ollama not reachable at ($host) — exiting 75 (transient)\n" | save --append $stderr_abs
        exit 75
    }

    # Model check
    let tags = (^curl -sf $"($host)/api/tags")
    if not ($tags | str contains $"\"($model)\"") {
        $"model ($model) not found — pull it with: ollama pull ($model)\n" | save --append $stderr_abs
        exit 75
    }

    let prompt_text = open --raw $prompt_abs
    let prompt_size = ($prompt_text | str length)
    $"running ($model) on ($prompt_size) byte prompt\n" | save --append $stderr_abs

    ^ollama run $model $prompt_text o> $stdout_abs e> $stderr_abs
    let rc = $env.LAST_EXIT_CODE

    # Apply structured file output if the model emits a JSON {"files": [...]} block
    let extract_py = '
import sys, json, re

text = open(sys.argv[1]).read()
candidates = re.findall(r"```json\s*(.*?)```", text, re.DOTALL)
candidates += re.findall(r"(\{[^{}]*\"files\"\s*:\s*\[.*?\].*?\})", text, re.DOTALL)
for candidate in candidates:
    try:
        obj = json.loads(candidate.strip())
        if isinstance(obj.get("files"), list):
            print(candidate.strip())
            break
    except Exception:
        continue
'

    let apply_py = '
import json, os, sys
data = json.loads(open(sys.argv[1]).read())
files = data.get("files", [])
for f in files:
    path = f.get("path", "").strip()
    content = f.get("content", "")
    if not path or path.startswith("/") or ".." in path:
        print(f"skipping unsafe path: {path!r}", file=sys.stderr)
        continue
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    with open(path, "w") as fh:
        fh.write(content)
    print(f"wrote {path} ({len(content)} bytes)", file=sys.stderr)
if files:
    os.makedirs("output", exist_ok=True)
    summary = "# Ollama File Output\n\n" + "\n".join(f"- {f[\"path\"]}" for f in files if f.get("path"))
    open("output/result.md", "w").write(summary)
'

    mkdir output
    let json_block = (do { ^python3 -c $extract_py $stdout_abs } | complete | get stdout | str trim)

    if ($json_block | str length) > 0 {
        "applying structured file output\n" | save --append $stderr_abs
        let tmpjson = (^mktemp /tmp/bop-ollama-files.XXXXXX.json)
        $json_block | save --force $tmpjson
        do { ^python3 -c $apply_py $tmpjson } | complete | get stderr | save --append $stderr_abs
        ^rm -f $tmpjson
    }

    exit $rc
}
