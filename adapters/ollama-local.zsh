#!/usr/bin/env zsh
set -euo pipefail

workdir="$1"; prompt_file="$2"; stdout_log="$3"; stderr_log="$4"

# Store original working directory
orig_dir="$(pwd)"

# Change to workdir for execution
cd "$workdir" || exit 1

# Convert paths relative to original directory
if [[ "$prompt_file" != /* ]]; then
    prompt_file="$orig_dir/$prompt_file"
fi
if [[ "$stdout_log" != /* ]]; then
    stdout_log="$orig_dir/$stdout_log"
fi
if [[ "$stderr_log" != /* ]]; then
    stderr_log="$orig_dir/$stderr_log"
fi

# Read the full prompt content
full_prompt="$(cat "$prompt_file")"

# Detect if this is an ideation/brainstorming task
if echo "$full_prompt" | grep -qi "ideation\|brainstorm\|enhancement\|innovation"; then
    # Use a more creative model for ideation
    model="qwen2.5:7b"
    system_prompt="You are an expert innovation consultant and systems thinker. Your task is to run a comprehensive ideation session. Think step-by-step, explore multiple angles, and provide structured, actionable insights. Use the following framework:
1. Current State Analysis
2. Problem Identification
3. Brainstorming (multiple ideas)
4. Feasibility Assessment
5. Prioritization Matrix
6. Implementation Roadmap
7. Success Metrics

Be thorough, creative, and practical in your analysis."
else
    # Use coding model for implementation tasks
    model="codellama:7b"
    # Extract the task content for coding
    spec_content=$(grep -A 50 "{{spec}}" "$prompt_file" | grep -v "{{spec}}" | grep -v "{{acceptance_criteria}}" | grep -v "Please implement" | grep -v "Focus on:" | grep -v "When complete" | head -20)
    system_prompt="You are a expert Rust programmer. Write clean, efficient code following best practices. Provide only the implementation without extensive explanations."
    full_prompt="Write Rust code for the following task:

$spec_content

Please provide only the Rust code implementation. No explanations needed."
fi

# Use Ollama with appropriate model and prompt
if [ "$model" = "qwen2.5:7b" ]; then
    echo "Running ideation session with $model..." >> "$stderr_log"
    ollama run "$model" "$system_prompt

$full_prompt" \
        > "$stdout_log" 2>> "$stderr_log"
else
    echo "Running coding task with $model..." >> "$stderr_log"
    ollama run "$model" "$full_prompt" \
        > "$stdout_log" 2>> "$stderr_log"
fi

rc=$?

# Ollama typically doesn't have rate limits, but check for common errors
if grep -qiE 'model not found|out of memory|connection refused' "$stderr_log"; then
  exit 75
fi

exit $rc
