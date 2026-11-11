# Open Cycling World – developer tasks (Linux).
# `make help` lists the targets.

# Use the rustup toolchain even if ~/.cargo/bin is not on PATH (Debian's
# system Rust is too old for Bevy 0.20).
export PATH := $(HOME)/.cargo/bin:$(PATH)

CARGO ?= cargo
WASM_CRATES := -p ocw-core -p ocw-sim -p ocw-protocol -p ocw-world
# Build: compiler, Bevy (audio, input, windowing), btleplug (D-Bus).
# Run/test: Vulkan driver + vulkaninfo, BlueZ, headless displays (Xvfb, weston),
# ffmpeg for videos, rsync/git for scripts/remote.sh.
APT_PACKAGES := build-essential pkg-config curl git rsync \
	libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev libxkbcommon-x11-0 \
	libdbus-1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
	mesa-vulkan-drivers vulkan-tools bluez \
	xvfb xauth weston ffmpeg
SCREENSHOT ?= /tmp/ocw.png
SCREENSHOT_WAYLAND ?= /tmp/ocw-wayland.png
VIDEO ?= /tmp/ocw.mp4
VIDEO_WAYLAND ?= /tmp/ocw-wayland.mp4

.DEFAULT_GOAL := help
.PHONY: help deps deps-linux deps-linux-packages deps-windows deps-macos deps-rust build release test lint fmt fmt-check \
	clippy wasm check run server screenshot screenshot-wayland smoke video video-wayland deploy-test clean clean-test-output \
	macos-app

help: ## Show this help
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*## "}; {printf "  \033[36m%-19s\033[0m %s\n", $$1, $$2}'

# Platform detection for `make deps` (Windows: GNU make from Git Bash/MSYS2).
ifeq ($(OS),Windows_NT)
  PLATFORM := windows
else ifeq ($(shell uname -s),Darwin)
  PLATFORM := macos
else
  PLATFORM := linux
endif

deps: deps-$(PLATFORM) ## Install everything for this platform (Linux, Windows or macOS)

deps-linux: deps-linux-packages deps-rust ## Linux: Debian/Ubuntu packages (asks for sudo) + Rust toolchain

deps-linux-packages: ## Linux: only the Debian/Ubuntu packages (sudo; also for other users' machines)
	sudo apt-get update
	sudo apt-get install -y $(APT_PACKAGES)
	@echo "For GPU access without a desktop session (remote/benchmark machines): sudo usermod -aG render,video \$$USER"

deps-windows: ## Windows: Git, VS Build Tools (C++), rustup via winget + Rust toolchain
	winget install --id Git.Git -e --accept-source-agreements --accept-package-agreements
	winget install --id Microsoft.VisualStudio.2022.BuildTools -e --accept-source-agreements --accept-package-agreements \
		--override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
	winget install --id Rustlang.Rustup -e --accept-source-agreements --accept-package-agreements
	rustup show active-toolchain || rustup toolchain install
	@echo "Open a new terminal so PATH includes rustup, Git and the MSVC tools."

deps-macos: ## macOS: Xcode Command Line Tools (+ ffmpeg if Homebrew is installed) + Rust toolchain
	@xcode-select -p >/dev/null 2>&1 || { xcode-select --install; \
		echo "Finish the Command Line Tools installer, then run 'make deps-macos' again."; exit 1; }
	@if command -v brew >/dev/null 2>&1; then brew install ffmpeg; \
		else echo "Homebrew not found: skipping ffmpeg (only needed for videos)."; fi
	$(MAKE) deps-rust

deps-rust: ## Linux/macOS: rustup (if missing) + the pinned toolchain incl. wasm target
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

check: lint test wasm ## fmt, clippy, tests, wasm (CI also runs the smoke tests)

run: ## Start the desktop client (needs a display)
	$(CARGO) run -p ocw-desktop

server: ## Run the headless server for 10 s with 3 test riders
	$(CARGO) run -p ocw-server -- --bots 3 --seconds 10 --fast

screenshot: ## Headless smoke test on X11 (Xvfb); writes $(SCREENSHOT)
	.claude/skills/run-ocw/drive.sh shot $(SCREENSHOT)

screenshot-wayland: ## Headless smoke test on Wayland (weston); writes $(SCREENSHOT_WAYLAND)
	.claude/skills/run-ocw/drive.sh shot-wayland $(SCREENSHOT_WAYLAND)

smoke: screenshot screenshot-wayland ## Both desktop smoke tests (X11 and Wayland)

video: ## Record a smooth 6 s test ride (X11, frame capture); writes $(VIDEO)
	.claude/skills/run-ocw/drive.sh video $(VIDEO)

video-wayland: ## Same on Wayland (weston); writes $(VIDEO_WAYLAND)
	.claude/skills/run-ocw/drive.sh video-wayland $(VIDEO_WAYLAND)

deploy-test: ## Build on MACHINE (default marge) and deploy for its kiosk (scripts/ocw-kiosk.sh)
	scripts/deploy-test.sh $(or $(MACHINE),marge)

# macOS app bundle (spec §11.3).
macos-app: ## macOS: Open Cycling World.app + DMG + zip in target/macos (ad-hoc signed)
	scripts/build-macos-app.sh

clean: ## Remove build artefacts
	$(CARGO) clean

clean-test-output: ## Delete the archived screenshots and videos in test-output/
	rm -rf test-output
