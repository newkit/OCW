# Open Cycling World – developer tasks (Linux).
# `make help` lists the targets.

# Use the rustup toolchain even if ~/.cargo/bin is not on PATH (Debian's
# system Rust is too old for Bevy 0.20).
export PATH := $(HOME)/.cargo/bin:$(PATH)

CARGO ?= cargo
WASM_CRATES := -p ocw-core -p ocw-sim -p ocw-protocol -p ocw-world
APT_PACKAGES := build-essential pkg-config curl \
	libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev libxkbcommon-x11-0 \
	libdbus-1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
	xvfb xauth mesa-vulkan-drivers
SCREENSHOT ?= /tmp/ocw.png

.DEFAULT_GOAL := help
.PHONY: help deps deps-system deps-rust build release test lint fmt fmt-check \
	clippy wasm check run server screenshot clean

help: ## Show this help
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*## "}; {printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}'

deps: deps-system deps-rust ## Install system packages (sudo) and the Rust toolchain

deps-system: ## Install the Debian/Ubuntu build and runtime packages (sudo)
	sudo apt-get update
	sudo apt-get install -y $(APT_PACKAGES)

deps-rust: ## Install rustup (if missing) and the pinned toolchain incl. wasm target
	@command -v rustup >/dev/null 2>&1 || \
		curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
		sh -s -- -y --no-modify-path --default-toolchain none
	rustup show active-toolchain || rustup toolchain install
	@echo "Toolchain ready. Add ~/.cargo/bin to PATH (e.g. '. \"\$$HOME/.cargo/env\"' in ~/.profile) to use cargo outside make."

build: ## Debug build of all crates and apps
	$(CARGO) build --workspace

release: ## Optimized build of ocw-desktop and ocw-server
	$(CARGO) build --release -p ocw-desktop -p ocw-server

test: ## Run all unit and integration tests (no hardware, network or GPU)
	$(CARGO) test --workspace

fmt: ## Format all code
	$(CARGO) fmt --all

fmt-check: ## Check formatting
	$(CARGO) fmt --all -- --check

clippy: ## Lint; warnings are errors (as in CI)
	$(CARGO) clippy --workspace --all-targets -- -D warnings

lint: fmt-check clippy ## fmt-check + clippy

wasm: ## Check that the core crates build for wasm32-unknown-unknown
	$(CARGO) check $(WASM_CRATES) --target wasm32-unknown-unknown

check: lint test wasm ## Everything CI checks on Linux (except the screenshot smoke test)

run: ## Start the desktop client (needs a display)
	$(CARGO) run -p ocw-desktop

server: ## Run the headless server for 10 s with 3 test riders
	$(CARGO) run -p ocw-server -- --bots 3 --seconds 10 --fast

screenshot: ## Headless smoke test under Xvfb; writes $(SCREENSHOT)
	.claude/skills/run-ocw/drive.sh shot $(SCREENSHOT)

clean: ## Remove build artefacts
	$(CARGO) clean
