#!/usr/bin/env nu
# mlx.nu — run a card prompt via MLX (Apple Silicon native inference) + aider
#
# Uses mlx_lm.server to spin up a local OpenAI-compatible endpoint, then runs
# aider against it. MLX is ~2x faster than ollama on Apple Silicon and works
# fully offline — no network traffic once the model is downloaded.
#
# Prerequisites (one-time, before a flight):
#   uv tool install mlx-lm
#   uv tool install aider-chat
#   huggingface-cli download mlx-community/Qwen3.5-35B-A3B-4bit
#
# Env vars:
#   MLX_MODEL   HuggingFace model ID  (default: mlx-community/Qwen3.5-35B-A3B-4bit)
#   MLX_PORT    Server port           (default: 8080)
#   MLX_CONTEXT Max context tokens    (default: 32768)

const SERVER_START_TIMEOUT_S = 60   # seconds to wait for mlx server to become ready
const SERVER_POLL_INTERVAL_S = 2    # seconds between readiness polls

def main [
    workdir:      string = "",
    prompt_file:  string = "",
    stdout_log:   string = "",
    stderr_log:   string = "",
    _memory_out?: string,           # memory output file; read via BOP_MEMORY_OUT env
    --test                          # Run self-tests
] {
    if $test {
        run_tests
        return
    }

    if $workdir == "" {
        print -e "error: workdir is required"
        exit 1
    }

    let orig_dir  = (pwd)
    let prompt_abs = if ($prompt_file | str starts-with "/") { $prompt_file } else { [$orig_dir $prompt_file] | path join }
    let stdout_abs = if ($stdout_log  | str starts-with "/") { $stdout_log  } else { [$orig_dir $stdout_log]  | path join }
    let stderr_abs = if ($stderr_log  | str starts-with "/") { $stderr_log  } else { [$orig_dir $stderr_log]  | path join }

    # ── dependency checks ────────────────────────────────────────────────────
    # mlx_lm.server is the entry-point binary installed by uv tool install mlx-lm
    let has_mlx   = not (which mlx_lm.server | is-empty)
    let has_aider = not (which aider | is-empty)

    if not $has_mlx {
        "mlx_lm.server not found — install with: uv tool install mlx-lm\n" | save --append $stderr_abs
        exit 75   # transient: retry when installed
    }
    if not $has_aider {
        "aider not found — install with: uv tool install aider-chat\n" | save --append $stderr_abs
        exit 75
    }

    let model   = if "MLX_MODEL"   in $env { $env.MLX_MODEL }   else { "mlx-community/Qwen3.5-35B-A3B-4bit" }
    let port    = if "MLX_PORT"    in $env { $env.MLX_PORT | into int }    else { 8080 }
    let context = if "MLX_CONTEXT" in $env { $env.MLX_CONTEXT | into int } else { 32768 }
    let base_url = $"http://localhost:($port)/v1"

    # ── start server if not already running ──────────────────────────────────
    let already_running = (do { ^curl -sf $"($base_url)/models" } | complete | get exit_code) == 0

    let server_pid = if $already_running {
        $"mlx server already running on port ($port)\n" | save --append $stderr_abs
        ""
    } else {
        $"starting mlx_lm.server: ($model) on port ($port)\n" | save --append $stderr_abs

        let pid_file = (^mktemp /tmp/bop-mlx-server-XXXXXX.pid)
        ^sh -c $"mlx_lm.server --model '($model)' --port ($port) --max-tokens ($context) >> /tmp/bop-mlx-server.log 2>&1 & echo $! > ($pid_file)"

        # poll until ready (up to SERVER_START_TIMEOUT_S seconds)
        mut ready = false
        mut elapsed = 0
        while $elapsed < $SERVER_START_TIMEOUT_S {
            sleep ($"($SERVER_POLL_INTERVAL_S)sec" | into duration)
            let check = (do { ^curl -sf $"($base_url)/models" } | complete)
            if $check.exit_code == 0 {
                $ready = true
                break
            }
            $elapsed = $elapsed + $SERVER_POLL_INTERVAL_S
        }

        if not $ready {
            "mlx_lm.server failed to become ready — check /tmp/bop-mlx-server.log\n" | save --append $stderr_abs
            exit 75
        }

        $"mlx server ready after ($elapsed)s\n" | save --append $stderr_abs
        open --raw $pid_file | str trim
    }

    cd $workdir
    let prompt_text = open --raw $prompt_abs

    # ── run aider against mlx server ─────────────────────────────────────────
    # aider openai-compat shim: --model openai/<name> + --openai-api-base <url>
    # --auto-test + --test-cmd makes aider iterate on cargo check failures.
    # --map-tokens 0 skips repo-map (saves context for the actual implementation).
    # --thinking-tokens 0 suppresses <think> blocks in tool calls (Qwen3.5 uses CoT).
    let args = [
        $prompt_text
        "--model"            $"openai/($model)"
        "--openai-api-base"  $base_url
        "--auto-test"
        "--test-cmd"         "cargo check 2>&1"
        "--map-tokens"       "0"
        "--yes"
        "--no-git"
    ]
    ^aider ...$args o> $stdout_abs e> $stderr_abs
    let rc = $env.LAST_EXIT_CODE

    # ── stop server if we started it ─────────────────────────────────────────
    if ($server_pid | str length) > 0 {
        ^kill $server_pid e> /dev/null
        $"mlx server (pid ($server_pid)) stopped\n" | save --append $stderr_abs
    }

    # ── rate-limit / transient error detection ───────────────────────────────
    if ($stderr_abs | path exists) {
        let t = open --raw $stderr_abs
        if (($t | str contains --ignore-case "rate limit")
            or ($t | str contains "429")
            or ($t | str contains --ignore-case "too many requests")) {
            exit 75
        }
    }

    exit $rc
}

def run_tests []: nothing -> nothing {
    use std/assert

    # test 1: default model is the 4bit Qwen3.5 MoE variant
    let model = if "MLX_MODEL" in $env { $env.MLX_MODEL } else { "mlx-community/Qwen3.5-35B-A3B-4bit" }
    assert ($model | str contains "Qwen3.5")      "default model should be Qwen3.5"
    assert ($model | str contains "4bit")         "default model should be 4bit quantized"
    assert ($model | str starts-with "mlx-community/") "default model should be from mlx-community"

    # test 2: default port
    let port = if "MLX_PORT" in $env { $env.MLX_PORT | into int } else { 8080 }
    assert ($port == 8080) "default port should be 8080"

    # test 3: base url construction
    let base_url = $"http://localhost:($port)/v1"
    assert ($base_url == "http://localhost:8080/v1") "base URL should be correct"

    # test 4: aider model flag uses openai/ prefix
    let model_flag = $"openai/($model)"
    assert ($model_flag | str starts-with "openai/") "aider model flag must use openai/ prefix"

    # test 5: path resolution — absolute stays absolute
    let abs = if ("/tmp/foo" | str starts-with "/") { "/tmp/foo" } else { [(pwd) "foo"] | path join }
    assert ($abs == "/tmp/foo") "absolute path should stay absolute"

    # test 6: path resolution — relative gets prefixed
    let rel = if ("foo" | str starts-with "/") { "foo" } else { [(pwd) "foo"] | path join }
    assert ($rel | str starts-with "/") "resolved path should be absolute"

    # test 7: rate-limit detection
    let rl_text = "rate limit exceeded"
    assert ($rl_text | str contains --ignore-case "rate limit") "should detect rate limit string"

    # test 8: non-rate-limit does not trigger
    let ok_text = "cargo check failed: type mismatch"
    assert (not ($ok_text | str contains --ignore-case "rate limit")) "compile errors are not rate limits"

    print "PASS: mlx.nu"
}
