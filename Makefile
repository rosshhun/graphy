.PHONY: build build-web build-all install install-with-vectors test clean dev serve help

# Default target
help:
	@echo "Graphy - Graph-powered code intelligence engine"
	@echo ""
	@echo "Build targets:"
	@echo "  make build          Build the graphy binary"
	@echo "  make build-web      Build the Svelte frontend"
	@echo "  make build-all      Build frontend + binary (production)"
	@echo "  make build-vectors  Build with vector embedding support"
	@echo "  make install        Install graphy to ~/.cargo/bin"
	@echo "  make install-vectors Install with vector embedding support"
	@echo ""
	@echo "Development:"
	@echo "  make dev            Start frontend dev server (hot reload)"
	@echo "  make test           Run all tests"
	@echo "  make test-vectors   Run tests including vector module"
	@echo "  make check          Run cargo check + clippy"
	@echo "  make clean          Clean build artifacts"
	@echo ""
	@echo "Usage (after install):"
	@echo "  cd your-project && graphy        Analyze + dashboard + watch"
	@echo "  cd your-project && graphy init   Set up Claude Code integration"
	@echo "  cd your-project && graphy open   Open dashboard"

# ── Build ────────────────────────────────────────────────────

build:
	cargo build --release

build-web:
	cd web && npm install && npm run build

build-all: build-web build

build-vectors:
	cargo build --release -p graphy-search --features vectors
	cargo build --release

# ── Install ──────────────────────────────────────────────────

install: build-web
	cargo install --path crates/graphy-cli

install-with-vectors: build-web
	cargo install --path crates/graphy-cli --features graphy-search/vectors

# ── Test ─────────────────────────────────────────────────────

test:
	cargo test --workspace

test-vectors:
	cargo test --workspace
	cargo test -p graphy-search --features vectors

check:
	cargo check --workspace
	cargo clippy --workspace -- -D warnings 2>/dev/null || true

# ── Development ──────────────────────────────────────────────

dev:
	@echo "Starting Vite dev server on :5173 (proxy API to :3000)..."
	@echo "Run 'graphy ui ./your-project' in another terminal first."
	cd web && npm run dev

# ── Convenience ──────────────────────────────────────────────

PATH ?= .

analyze:
	cargo run --release -- analyze $(PATH)

serve:
	cargo run --release -- serve $(PATH)

ui:
	cargo run --release -- ui $(PATH)

# ── Clean ────────────────────────────────────────────────────

clean:
	cargo clean
	rm -rf web/dist web/node_modules
