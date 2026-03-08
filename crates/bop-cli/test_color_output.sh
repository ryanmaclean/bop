#!/bin/bash
# Quick test to verify ANSI color codes in bop list output

output=$(cargo run --bin bop -- list --state all 2>/dev/null)

if echo "$output" | grep -q $'\x1b\[38;5;'; then
    echo "✓ ANSI color codes detected in output"
    exit 0
else
    echo "✗ No ANSI color codes found"
    echo "Output sample:"
    echo "$output" | head -5
    exit 1
fi
