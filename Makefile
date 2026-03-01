.PHONY: test build clean lint

# Build the project
build:
	cargo build

# Run tests
test:
	cargo test

# Run clippy for linting
lint:
	cargo clippy -- -D warnings

# Clean build artifacts
clean:
	cargo clean

# Format code
fmt:
	cargo fmt

# Check formatting
check-fmt:
	cargo fmt --check

# Run all checks
check: test lint check-fmt

# Install to system
install: build
	sudo cp target/debug/bop /usr/local/bin/bop

# Development setup
dev:
	cargo watch -x 'run -- bop dispatcher --once'
