#!/bin/bash
# Verification script for HTML tooltip implementation

echo "=== HTML Tooltip Verification ==="
echo ""
echo "Checking gantt.rs for tooltip implementation..."
echo ""

# Extract the tooltip generation code
grep -A 8 "let tip = format!" crates/bop-cli/src/gantt.rs

echo ""
echo "=== Required Fields Check ==="
echo ""

required_fields=("id:" "stage:" "provider:" "duration:" "tokens:" "cost:")
all_present=true

for field in "${required_fields[@]}"; do
    if grep -q "\"$field" crates/bop-cli/src/gantt.rs; then
        echo "✓ $field found"
    else
        echo "✗ $field MISSING"
        all_present=false
    fi
done

echo ""
if [ "$all_present" = true ]; then
    echo "✓ All required tooltip fields are present!"
    exit 0
else
    echo "✗ Some required fields are missing"
    exit 1
fi
