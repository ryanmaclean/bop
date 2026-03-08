#!/bin/bash

# Auto-Build Environment Setup for Spec 026
# Storage resilience: JSONL WAL + checksum

set -e

echo "========================================"
echo "Starting Development Environment"
echo "========================================"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Project root
PROJECT_ROOT="/Users/studio/bop"

echo ""
echo "Project: bop (Rust workspace)"
echo "Crates: bop-core, bop-cli"
echo "Working Directory: $PROJECT_ROOT"
echo ""

# ============================================
# VERIFY ENVIRONMENT
# ============================================

echo "Checking Rust toolchain..."
if ! command -v cargo &> /dev/null; then
    echo -e "${RED}cargo not found - install Rust toolchain${NC}"
    exit 1
fi
echo -e "${GREEN}✓ cargo available${NC}"

echo "Checking rustfmt..."
if ! command -v rustfmt &> /dev/null; then
    echo -e "${YELLOW}⚠ rustfmt not found - install via 'rustup component add rustfmt'${NC}"
fi

echo "Checking clippy..."
if ! command -v cargo-clippy &> /dev/null; then
    echo -e "${YELLOW}⚠ clippy not found - install via 'rustup component add clippy'${NC}"
fi

# ============================================
# BUILD PROJECT
# ============================================

echo ""
echo "Building bop workspace..."
cd "$PROJECT_ROOT"
cargo build

if [ $? -eq 0 ]; then
    echo -e "${GREEN}✓ Build successful${NC}"
else
    echo -e "${RED}✗ Build failed${NC}"
    exit 1
fi

# ============================================
# SUMMARY
# ============================================

echo ""
echo "========================================"
echo "Environment Ready!"
echo "========================================"
echo ""
echo "Workspace:"
echo "  Root:     $PROJECT_ROOT"
echo "  Binary:   target/debug/bop"
echo ""
echo "Development Commands:"
echo "  Build:    cargo build"
echo "  Test:     cargo test"
echo "  Lint:     cargo clippy -- -D warnings"
echo "  Format:   cargo fmt"
echo "  Check:    make check"
echo ""
echo "Spec 026 Implementation:"
echo "  Phase 1:  Add blake3 dependency"
echo "  Phase 2:  Implement JSONL WAL (append_event)"
echo "  Phase 3:  Add checksum field and validation"
echo "  Phase 4:  Wire up dispatcher event logging"
echo "  Phase 5:  Write documentation"
echo "  Phase 6:  Final verification (make check)"
echo ""
