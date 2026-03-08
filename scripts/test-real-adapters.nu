#!/usr/bin/env nu
# Test real adapters (claude, ollama, codex) end-to-end through the dispatcher pipeline.
#
# Usage:
#   nu --no-config-file scripts/test-real-adapters.nu [--adapter <name>] [--timeout <seconds>] [--test]
#
# Flags:
#   --adapter <name>   Adapter to test (default: all). Options: all, claude, ollama, codex
#   --timeout <seconds> Timeout in seconds for each adapter test (default: 120)
#   --test             Run internal self-tests
#   --help             Show this help message

def main [
  --adapter: string = "all"  # Adapter to test (all, claude, ollama, codex)
  --timeout: int = 120       # Timeout in seconds
  --test                      # Run internal self-tests
  --help                      # Show help message
] {
  if $help {
    print "Test real adapters (claude, ollama, codex) end-to-end through the dispatcher pipeline."
    print ""
    print "Usage:"
    print "  nu --no-config-file scripts/test-real-adapters.nu [OPTIONS]"
    print ""
    print "Options:"
    print "  --adapter <name>     Adapter to test (default: all)"
    print "                       Options: all, claude, ollama, codex"
    print "  --timeout <seconds>  Timeout in seconds for each test (default: 120)"
    print "  --test               Run internal self-tests"
    print "  --help               Show this help message"
    print ""
    print "Examples:"
    print "  nu --no-config-file scripts/test-real-adapters.nu"
    print "  nu --no-config-file scripts/test-real-adapters.nu --adapter claude"
    print "  nu --no-config-file scripts/test-real-adapters.nu --adapter ollama --timeout 60"
    print "  nu --no-config-file scripts/test-real-adapters.nu --test"
    return
  }

  if $test {
    run_tests
    return
  }

  # Build bop binary before running tests
  print "Building bop binary..."
  build_bop
  let bop = (get_bop_bin)
  if not ($bop | path exists) {
    print "ERROR: bop binary not found after build"
    exit 1
  }

  # Determine which adapters to test
  let adapters_to_test = if $adapter == "all" {
    ["claude", "ollama", "codex"]
  } else {
    [$adapter]
  }

  # Track results
  mut results = []
  mut all_passed = true

  # Test each adapter
  for adapter_name in $adapters_to_test {
    # Check if adapter script exists
    let adapter_path = (get_adapter_path $adapter_name)
    if not ($adapter_path | path exists) {
      let result = {adapter: $adapter_name, success: false, reason: "adapter script not found"}
      $results = ($results | append [$result])
      $all_passed = false
      continue
    }

    # Run adapter test
    let test_result = (run_adapter_test $adapter_name $timeout)
    $results = ($results | append [$test_result])

    if not $test_result.success {
      $all_passed = false
    }
  }

  # Format and display results
  print ""
  print "Test Results:"
  print "============="
  for result in $results {
    let symbol = if $result.success { "✓" } else { "⊘" }
    let status = if $result.success { "PASS" } else { "FAIL" }
    print $"($symbol) ($result.adapter): ($status)"
    if not $result.success and "reason" in $result {
      print $"  Reason: ($result.reason)"
    }
  }

  # Exit with appropriate code
  if not $all_passed {
    exit 1
  }
}

# Run a single adapter test
def run_adapter_test [
  adapter: string  # Adapter name to test
  timeout_sec: int # Timeout in seconds
]: nothing -> record<adapter: string, success: bool, reason: string> {
  let card_id = $"test-($adapter)-(date now | format date '%s')"

  # Create test setup
  let setup = try {
    create_test_setup $adapter
  } catch { |err|
    return {adapter: $adapter, success: false, reason: $"setup failed: ($err.msg)"}
  }

  # Write template
  try {
    write_template $setup.cards_dir "implement" $adapter
  } catch { |err|
    cleanup_test_setup $setup.temp_dir
    return {adapter: $adapter, success: false, reason: $"write template failed: ($err.msg)"}
  }

  # Create test card
  try {
    create_test_card $setup.cards_dir "implement" $card_id
  } catch { |err|
    cleanup_test_setup $setup.temp_dir
    return {adapter: $adapter, success: false, reason: $"create card failed: ($err.msg)"}
  }

  # Start dispatcher
  let log_file = ($setup.temp_dir | path join "dispatcher.log")
  start_dispatcher $setup.cards_dir $adapter $log_file

  # Give dispatcher a moment to start
  sleep 500ms

  # Wait for card completion
  let completion = (wait_for_card_completion $setup.cards_dir $card_id $timeout_sec)

  # Stop dispatcher
  stop_dispatcher $setup.cards_dir

  # Check results
  let result = if not $completion.success {
    {adapter: $adapter, success: false, reason: $"card stuck in ($completion.state) state"}
  } else {
    let assertion = (assert_result $setup.cards_dir $card_id)
    if $assertion.success {
      {adapter: $adapter, success: true, reason: "test passed"}
    } else {
      {adapter: $adapter, success: false, reason: $assertion.message}
    }
  }

  # Cleanup (keep temp dir on failure for debugging)
  if not $result.success {
    print $"DEBUG: temp_dir kept at: ($setup.temp_dir)"
  } else {
    cleanup_test_setup $setup.temp_dir
  }

  $result
}

# Check if a specific adapter command is available in PATH
def is_adapter_available [
  adapter: string  # Adapter name (claude, ollama, or codex)
]: nothing -> bool {
  # Check if command exists
  let cmd_result = (do { ^which $adapter } | complete)
  if $cmd_result.exit_code != 0 {
    return false
  }

  # Additional checks per adapter
  if $adapter == "ollama" {
    # Check if ollama server is running
    let server_result = (do { ^curl -sf http://localhost:11434/api/tags } | complete)
    return ($server_result.exit_code == 0)
  } else if $adapter == "codex" {
    # Check if OPENAI_API_KEY is set
    return ("OPENAI_API_KEY" in $env)
  } else {
    # claude or other adapters: just check command existence
    return true
  }
}

# Get list of available adapters from the standard set
def get_available_adapters []: nothing -> list<string> {
  ["claude", "ollama", "codex"]
  | where { |adapter| (is_adapter_available $adapter) }
}

# Get path to the bop binary
def get_bop_bin []: nothing -> path {
  let root = ($env.CURRENT_FILE | path dirname | path dirname)
  $root | path join "target" "debug" "bop"
}

# Get path to the repository root
def get_repo_root []: nothing -> path {
  $env.CURRENT_FILE | path dirname | path dirname
}

# Get path to an adapter script
def get_adapter_path [
  adapter: string  # Adapter name (e.g., "mock", "claude", "ollama")
]: nothing -> path {
  let root = (get_repo_root)
  $root | path join "adapters" $"($adapter).nu"
}

# Build the bop binary before running tests
def build_bop []: nothing -> nothing {
  let root = (get_repo_root)
  let result = (do {
    cargo build
  } | complete)

  if $result.exit_code != 0 {
    error make {
      msg: "Failed to build bop binary"
      label: {
        text: $"cargo build failed with exit code ($result.exit_code)"
        span: (metadata $result).span
      }
    }
  }
}

# Create a test setup with temp directory, .cards structure, and providers.json
def create_test_setup [
  adapter: string  # Primary adapter to use (e.g., "mock", "claude")
]: nothing -> record<cards_dir: path, temp_dir: path> {
  # Create temp directory
  let temp_dir = (mktemp -d)
  let cards_dir = ($temp_dir | path join ".cards")

  # Initialize .cards directory
  let bop = (get_bop_bin)
  let init_result = (do {
    ^$bop --cards-dir $cards_dir init
  } | complete)

  if $init_result.exit_code != 0 {
    error make {
      msg: "Failed to initialize .cards directory"
      label: {
        text: $"bop init failed with exit code ($init_result.exit_code)"
        span: (metadata $init_result).span
      }
    }
  }

  # Write providers.json
  write_providers $cards_dir $adapter

  {cards_dir: $cards_dir, temp_dir: $temp_dir}
}

# Write providers.json with the specified adapter
def write_providers [
  cards_dir: path   # Path to .cards directory
  adapter: string   # Adapter name (e.g., "mock", "claude")
]: nothing -> nothing {
  let adapter_path = (get_adapter_path $adapter)
  let adapter_str = ($adapter_path | str replace --all '\\' '\\\\')

  let providers = {
    providers: {
      $adapter: {
        command: $adapter_str
        rate_limit_exit: 75
      }
    }
  }

  $providers | to json | save --force ($cards_dir | path join "providers.json")
}

# Write a template card structure
def write_template [
  cards_dir: path   # Path to .cards directory
  template: string  # Template name (e.g., "implement")
  adapter: string = "mock"  # Adapter name to use in provider_chain (e.g., "claude", "mock")
]: nothing -> nothing {
  let tdir = ($cards_dir | path join "templates" $"($template).bop")

  mkdir ($tdir | path join "logs")
  mkdir ($tdir | path join "output")

  let meta = {
    id: "t"
    created: "2026-03-01T00:00:00Z"
    stage: "implement"
    provider_chain: [$adapter]
    stages: {}
    acceptance_criteria: []
  }

  $meta | to json | save --force ($tdir | path join "meta.json")

  let spec_content = "Create a file at output/result.md containing exactly the text: hello from adapter

Use the Write tool to create files. Create the output/ directory first if needed.
Do not write any other files. Do not explain anything."

  $spec_content | save --force ($tdir | path join "spec.md")
  "{{spec}}\n" | save --force ($tdir | path join "prompt.md")
}

# Create a test card from a template
def create_test_card [
  cards_dir: path   # Path to .cards directory
  template: string  # Template name (e.g., "implement")
  card_id: string   # Card ID (e.g., "test-job-1")
]: nothing -> nothing {
  let bop = (get_bop_bin)
  let result = (do {
    ^$bop --cards-dir $cards_dir new $template $card_id
  } | complete)

  if $result.exit_code != 0 {
    error make {
      msg: "Failed to create test card"
      label: {
        text: $"bop new failed with exit code ($result.exit_code)"
        span: (metadata $result).span
      }
    }
  }
}

# Find a card by ID in a state directory (handles glyph-prefixed names)
def find_card_in [
  cards_dir: path   # Path to .cards directory
  state: string     # State directory (e.g., "pending", "running", "done")
  card_id: string   # Card ID to find
]: nothing -> path {
  let state_dir = ($cards_dir | path join $state)
  let suffix = $"-($card_id).bop"
  let exact = $"($card_id).bop"

  # Check for exact match first
  let exact_path = ($state_dir | path join $exact)
  if ($exact_path | path exists) {
    return $exact_path
  }

  # Look for glyph-prefixed names
  let entries = (ls $state_dir | where name ends-with $suffix)
  if ($entries | length) > 0 {
    return ($entries | first | get name)
  }

  # Return non-existent path for better error messages
  $exact_path
}

# Cleanup test setup by removing temp directory
def cleanup_test_setup [
  temp_dir: path  # Temp directory to remove
]: nothing -> nothing {
  if ($temp_dir | path exists) {
    rm -rf $temp_dir
  }
}

# Start dispatcher process in the background
def start_dispatcher [
  cards_dir: path   # Path to .cards directory
  adapter: string   # Adapter name (e.g., "mock", "claude")
  log_file: path    # Path to log file
]: nothing -> nothing {
  let bop = (get_bop_bin)
  let adapter_path = (get_adapter_path $adapter)

  # Kill any existing dispatcher for this cards_dir
  do { ^pkill -f $"bop --cards-dir ($cards_dir) dispatcher" } | complete

  # Start dispatcher in background with nohup (unset CLAUDECODE to allow bop to run)
  ^sh -c $"unset CLAUDECODE; nohup ($bop) --cards-dir ($cards_dir) dispatcher --adapter ($adapter_path) --vcs-engine git_gt --poll-ms 100 --reap-ms 500 >> ($log_file) 2>&1 &"
}

# Stop dispatcher process for a given cards directory
def stop_dispatcher [
  cards_dir: path  # Path to .cards directory
]: nothing -> nothing {
  do { ^pkill -f $"bop --cards-dir ($cards_dir) dispatcher" } | complete | ignore
}

# Poll for card to reach a target state with timeout
def poll_for_state [
  cards_dir: path   # Path to .cards directory
  card_id: string   # Card ID to poll
  target_state: string  # Target state (e.g., "running", "done")
  timeout_sec: int  # Timeout in seconds
]: nothing -> bool {
  let start_time = (date now | format date "%s" | into int)
  let poll_interval_ms = 200ms

  loop {
    let current_time = (date now | format date "%s" | into int)
    let elapsed = ($current_time - $start_time)

    if $elapsed >= $timeout_sec {
      return false
    }

    # Check if card exists in target state
    let card_path = (find_card_in $cards_dir $target_state $card_id)
    if ($card_path | path exists) {
      return true
    }

    sleep $poll_interval_ms
  }

  false
}

# Wait for card to transition through states: pending -> running -> done
def wait_for_card_completion [
  cards_dir: path   # Path to .cards directory
  card_id: string   # Card ID to wait for
  timeout_sec: int  # Total timeout in seconds
]: nothing -> record<success: bool, state: string> {
  let start_time = (date now | format date "%s" | into int)

  # Wait for card to move from pending to running
  let running_timeout = ($timeout_sec // 2)
  let running_result = (poll_for_state $cards_dir $card_id "running" $running_timeout)

  if not $running_result {
    return {success: false, state: "pending"}
  }

  # Calculate remaining timeout
  let current_time = (date now | format date "%s" | into int)
  let elapsed = ($current_time - $start_time)
  let remaining = ($timeout_sec - $elapsed)

  if $remaining <= 0 {
    return {success: false, state: "running"}
  }

  # Wait for card to move from running to done
  let done_result = (poll_for_state $cards_dir $card_id "done" $remaining)

  if not $done_result {
    return {success: false, state: "running"}
  }

  return {success: true, state: "done"}
}

# Assert that a card's result file exists and is non-empty
def assert_result [
  cards_dir: path   # Path to .cards directory
  card_id: string   # Card ID to check
]: nothing -> record<success: bool, message: string> {
  let card_path = (find_card_in $cards_dir "done" $card_id)

  if not ($card_path | path exists) {
    return {success: false, message: $"Card ($card_id) not found in done directory"}
  }

  let result_path = ($card_path | path join "output" "result.md")

  if not ($result_path | path exists) {
    return {success: false, message: $"Result file not found at ($result_path)"}
  }

  let result_content = (open --raw $result_path)
  let content_length = ($result_content | str length)

  if $content_length == 0 {
    return {success: false, message: "Result file is empty"}
  }

  return {success: true, message: $"Result file exists and contains ($content_length) bytes"}
}

def run_tests [] {
  use std/assert

  # Test: CLI argument defaults
  let default_adapter = "all"
  let default_timeout = 120
  assert equal $default_adapter "all"
  assert equal $default_timeout 120

  # Test: Valid adapter names
  let valid_adapters = ["all", "claude", "ollama", "codex"]
  assert (($valid_adapters | length) == 4) "should have 4 valid adapter options"
  assert ("all" in $valid_adapters) "all should be a valid adapter"
  assert ("claude" in $valid_adapters) "claude should be a valid adapter"

  # Test: is_adapter_available with a command that definitely exists
  let sh_available = (is_adapter_available "sh")
  assert $sh_available "sh should be available"

  # Test: is_adapter_available with a command that definitely doesn't exist
  let fake_available = (is_adapter_available "this-command-definitely-does-not-exist-12345")
  assert (not $fake_available) "fake command should not be available"

  # Test: get_available_adapters returns a list
  let available = (get_available_adapters)
  assert (($available | describe) =~ "list") "get_available_adapters should return a list"

  # Test: get_available_adapters returns only valid adapter names
  let available_set = ["claude", "ollama", "codex"]
  for adapter in $available {
    assert ($adapter in $available_set) $"($adapter) should be a valid adapter name"
  }

  # Test: get_repo_root returns a path
  let root = (get_repo_root)
  assert (($root | describe) =~ "string") "get_repo_root should return a path string"
  assert ($root | path exists) "repo root should exist"

  # Test: get_bop_bin returns a path
  let bop_path = (get_bop_bin)
  assert (($bop_path | describe) =~ "string") "get_bop_bin should return a path string"
  assert ($bop_path | str contains "target") "bop path should contain 'target'"
  assert ($bop_path | str contains "debug") "bop path should contain 'debug'"

  # Test: get_adapter_path returns a path
  let mock_path = (get_adapter_path "mock")
  assert (($mock_path | describe) =~ "string") "get_adapter_path should return a path string"
  assert ($mock_path | str contains "adapters") "adapter path should contain 'adapters'"
  assert ($mock_path | str contains "mock.nu") "mock adapter path should end with 'mock.nu'"

  # Test: build_bop (this will actually build the binary)
  print "Building bop binary for tests..."
  build_bop
  let bop = (get_bop_bin)
  assert ($bop | path exists) "bop binary should exist after build"

  # Test: create_test_setup creates temp dir and initializes .cards
  print "Testing create_test_setup..."
  let setup = (create_test_setup "mock")
  assert ($setup.temp_dir | path exists) "temp_dir should exist"
  assert ($setup.cards_dir | path exists) ".cards directory should exist"
  assert (($setup.cards_dir | path join "providers.json") | path exists) "providers.json should exist"
  assert (($setup.cards_dir | path join "pending") | path exists) "pending directory should exist"
  assert (($setup.cards_dir | path join "running") | path exists) "running directory should exist"
  assert (($setup.cards_dir | path join "done") | path exists) "done directory should exist"

  # Test: write_template creates template structure
  print "Testing write_template..."
  write_template $setup.cards_dir "implement"
  let template_dir = ($setup.cards_dir | path join "templates" "implement.bop")
  assert ($template_dir | path exists) "template directory should exist"
  assert (($template_dir | path join "meta.json") | path exists) "template meta.json should exist"
  assert (($template_dir | path join "spec.md") | path exists) "template spec.md should exist"
  assert (($template_dir | path join "prompt.md") | path exists) "template prompt.md should exist"
  assert (($template_dir | path join "logs") | path exists) "template logs directory should exist"
  assert (($template_dir | path join "output") | path exists) "template output directory should exist"

  # Test: create_test_card creates a card
  print "Testing create_test_card..."
  create_test_card $setup.cards_dir "implement" "test-job-1"
  let card = (find_card_in $setup.cards_dir "pending" "test-job-1")
  assert ($card | path exists) "test card should exist in pending"

  # Test: find_card_in finds cards with and without glyphs
  let found_card = (find_card_in $setup.cards_dir "pending" "test-job-1")
  assert ($found_card | path exists) "find_card_in should find the card"
  assert ($found_card | str contains "test-job-1") "found card path should contain card ID"

  # Test: cleanup_test_setup removes temp directory
  print "Testing cleanup_test_setup..."
  cleanup_test_setup $setup.temp_dir
  assert (not ($setup.temp_dir | path exists)) "temp_dir should be removed after cleanup"

  # Test: dispatcher execution and polling logic
  print "Testing dispatcher execution and polling..."
  let test_setup = (create_test_setup "mock")
  write_template $test_setup.cards_dir "implement"
  create_test_card $test_setup.cards_dir "implement" "test-dispatcher-1"

  # Start dispatcher
  let log_file = ($test_setup.temp_dir | path join "dispatcher.log")
  start_dispatcher $test_setup.cards_dir "mock" $log_file

  # Give dispatcher a moment to start
  sleep 500ms

  # Test: poll_for_state with quick timeout (should timeout on non-existent state)
  let poll_result = (poll_for_state $test_setup.cards_dir "test-dispatcher-1" "failed" 1)
  assert (not $poll_result) "poll_for_state should timeout when card doesn't reach state"

  # Test: wait_for_card_completion
  let completion_result = (wait_for_card_completion $test_setup.cards_dir "test-dispatcher-1" 30)
  assert $completion_result.success "card should complete successfully with mock adapter"
  assert ($completion_result.state == "done") "card should be in done state"

  # Verify card is actually in done directory
  let done_card = (find_card_in $test_setup.cards_dir "done" "test-dispatcher-1")
  assert ($done_card | path exists) "card should exist in done directory"

  # Mock adapter doesn't create output/result.md, so create it manually for testing
  mkdir ($done_card | path join "output")
  "test output from mock adapter" | save ($done_card | path join "output" "result.md")

  # Test: assert_result with successful card
  print "Testing assert_result..."
  let result = (assert_result $test_setup.cards_dir "test-dispatcher-1")
  assert $result.success "assert_result should succeed for completed card with output"
  assert ($result.message | str contains "bytes") "result message should contain byte count"

  # Test: assert_result with non-existent card
  let missing_result = (assert_result $test_setup.cards_dir "nonexistent-card-123")
  assert (not $missing_result.success) "assert_result should fail for non-existent card"
  assert ($missing_result.message | str contains "not found") "error message should mention card not found"

  # Test: assert_result with empty result file
  let test_setup2 = (create_test_setup "mock")
  write_template $test_setup2.cards_dir "implement"
  create_test_card $test_setup2.cards_dir "implement" "test-empty-result"

  # Manually move card to done and create empty result file
  let pending_card = (find_card_in $test_setup2.cards_dir "pending" "test-empty-result")
  let done_dir = ($test_setup2.cards_dir | path join "done")
  let done_card_path = ($done_dir | path join ($pending_card | path basename))
  mv $pending_card $done_card_path

  # Create empty result file
  mkdir ($done_card_path | path join "output")
  "" | save ($done_card_path | path join "output" "result.md")

  let empty_result = (assert_result $test_setup2.cards_dir "test-empty-result")
  assert (not $empty_result.success) "assert_result should fail for empty result file"
  assert ($empty_result.message | str contains "empty") "error message should mention empty file"

  # Cleanup second test setup
  cleanup_test_setup $test_setup2.temp_dir

  # Stop dispatcher
  stop_dispatcher $test_setup.cards_dir

  # Cleanup
  cleanup_test_setup $test_setup.temp_dir

  print "PASS: test-real-adapters.nu"
}
