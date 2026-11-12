# Open Cycling World – developer tasks (Linux).
# `make help` lists the targets.

# Use the rustup toolchain even if ~/.cargo/bin is not on PATH (Debian's
# system Rust is too old for Bevy 0.20).
export PATH := $(HOME)/.cargo/bin:$(PATH)

CARGO ?= cargo
WASM_CRATES := -p ocw-core -p ocw-sim -p ocw-protocol -p ocw-world -p ocw-voice
# Voice chat (spec §7): the LiveKit SDK's bindings need clang >= 21 on Linux
# (`make deps-llvm-debian`). With clang-21 installed, `build` and `release`
# include the desktop's `voice` feature; without it they build as before.
# VOICE=0 turns it off, VOICE=1 insists on it.
VOICE ?= $(if $(shell command -v clang++-21 2>/dev/null),1,0)
ifeq ($(VOICE),1)
VOICE_ENV := CC=clang-21 CXX=clang++-21
VOICE_FEATURES := --features ocw-desktop/voice
endif
# Build: compiler, Bevy (audio, input, windowing), btleplug (D-Bus), GLib / GIO
# headers for the voice feature (webrtc-sys always needs them on Linux).
# Run/test: Vulkan driver + vulkaninfo, BlueZ, headless displays (Xvfb, weston),
# ffmpeg for videos, rsync/git for scripts/remote.sh. FUSE 2 (to run
# AppImages such as appimagetool) comes separately in deps-linux-debian:
# libfuse2t64 on Debian 13 / Ubuntu 24.04, libfuse2 before.
APT_PACKAGES := build-essential pkg-config curl git rsync \
	libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev libxkbcommon-x11-0 \
	libdbus-1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
	mesa-vulkan-drivers vulkan-tools bluez \
	xvfb xauth weston ffmpeg libglib2.0-dev
# The same for Fedora / RHEL (dnf). ffmpeg is handled separately in
# deps-linux-fedora: Fedora's ffmpeg-free conflicts with RPM Fusion's ffmpeg.
DNF_PACKAGES := gcc gcc-c++ pkgconf-pkg-config curl git rsync \
	alsa-lib-devel systemd-devel wayland-devel libxkbcommon-devel libxkbcommon-x11-devel \
	dbus-devel libX11-devel libXcursor-devel libXrandr-devel libXi-devel libxcb-devel \
	mesa-vulkan-drivers vulkan-tools bluez \
	xorg-x11-server-Xvfb xorg-x11-xauth weston fuse-libs glib2-devel
# The same for Arch / Manjaro (pacman); the Vulkan driver depends on the GPU
# (vulkan-radeon, vulkan-intel, nvidia-utils), mesa covers OpenGL.
PACMAN_PACKAGES := base-devel pkgconf curl git rsync \
	alsa-lib systemd-libs wayland libxkbcommon libxkbcommon-x11 \
	dbus libx11 libxcursor libxrandr libxi libxcb \
	mesa vulkan-icd-loader vulkan-tools bluez bluez-utils \
	xorg-server-xvfb xorg-xauth weston ffmpeg fuse2 glib2
# Linux distribution family from /etc/os-release (ID, then ID_LIKE):
# debian (Debian, Ubuntu, Mint, …), fedora (Fedora, RHEL, Rocky, Alma, …),
# arch (Arch, Manjaro, EndeavourOS, …), or unknown.
OS_RELEASE_IDS := $(shell . /etc/os-release 2>/dev/null; echo $$ID $$ID_LIKE)
ifneq ($(filter debian ubuntu,$(OS_RELEASE_IDS)),)
  LINUX_FAMILY := debian
else ifneq ($(filter fedora rhel centos,$(OS_RELEASE_IDS)),)
  LINUX_FAMILY := fedora
else ifneq ($(filter arch archlinux manjaro,$(OS_RELEASE_IDS)),)
  LINUX_FAMILY := arch
else
  LINUX_FAMILY := unknown
endif
# Clang for the voice feature (LiveKit's webrtc-sys, spec §7) needs a newer
# version than Debian / Ubuntu ship (Debian 13: clang 19): installed from the
# LLVM project's apt repository (https://apt.llvm.org) by deps-llvm-debian.
LLVM_VERSION ?= 21
LLVM_APT_KEY_URL ?= https://apt.llvm.org/llvm-snapshot.gpg.key
LLVM_APT_KEYRING ?= /etc/apt/keyrings/apt.llvm.org.asc
LLVM_PACKAGES := clang-$(LLVM_VERSION) lld-$(LLVM_VERSION) libclang-$(LLVM_VERSION)-dev
# appimagetool for scripts/build-appimage.sh (user-level download, no sudo).
APPIMAGETOOL ?= $(HOME)/.local/bin/appimagetool
APPIMAGETOOL_URL ?= https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$(shell uname -m).AppImage
SCREENSHOT ?= /tmp/ocw.png
SCREENSHOT_WAYLAND ?= /tmp/ocw-wayland.png
VIDEO ?= /tmp/ocw.mp4
VIDEO_WAYLAND ?= /tmp/ocw-wayland.mp4

.DEFAULT_GOAL := help
.PHONY: help deps deps-linux deps-linux-packages deps-linux-debian deps-llvm-debian deps-linux-fedora deps-linux-arch deps-linux-unknown deps-appimage deps-windows deps-macos deps-rust build release test lint fmt fmt-check \
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

deps-linux: deps-linux-packages deps-rust deps-appimage ## Linux: system packages (detects Debian/Ubuntu, Fedora, Arch; sudo) + Rust + appimagetool

deps-linux-packages: deps-linux-$(LINUX_FAMILY) ## Linux: only the system packages for this distribution (sudo)
	@echo "For GPU access without a desktop session (remote/benchmark machines): sudo usermod -aG render,video \$$USER"

deps-linux-debian: ## Linux: Debian / Ubuntu packages (apt), incl. clang from apt.llvm.org (deps-llvm-debian)
	sudo apt-get update
	sudo apt-get install -y $(APT_PACKAGES)
	sudo apt-get install -y libfuse2t64 || sudo apt-get install -y libfuse2
	$(MAKE) deps-llvm-debian

deps-llvm-debian: ## Linux: clang, lld, libclang (LLVM_VERSION, default 21) from the LLVM apt repository (Debian / Ubuntu; sudo)
	@set -e; . /etc/os-release; codename=$${VERSION_CODENAME:-}; \
	if [ -z "$$codename" ]; then echo "No VERSION_CODENAME in /etc/os-release" >&2; exit 1; fi; \
	if [ "$$codename" = sid ]; then path=unstable; suite=llvm-toolchain-$(LLVM_VERSION); \
	else path=$$codename; suite=llvm-toolchain-$$codename-$(LLVM_VERSION); fi; \
	if ! curl --proto '=https' --tlsv1.2 -fsSI "https://apt.llvm.org/$$path/dists/$$suite/Release" >/dev/null; then \
		echo "apt.llvm.org has no $$suite for $$codename: install clang >= $(LLVM_VERSION) by hand" >&2; exit 1; fi; \
	sudo install -d -m 0755 "$(dir $(LLVM_APT_KEYRING))"; \
	curl --proto '=https' --tlsv1.2 -fsSL "$(LLVM_APT_KEY_URL)" | sudo tee "$(LLVM_APT_KEYRING)" >/dev/null; \
	echo "deb [signed-by=$(LLVM_APT_KEYRING)] https://apt.llvm.org/$$path/ $$suite main" | \
		sudo tee /etc/apt/sources.list.d/llvm-$(LLVM_VERSION).list >/dev/null; \
	sudo apt-get update; \
	sudo apt-get install -y $(LLVM_PACKAGES); \
	clang-$(LLVM_VERSION) --version | head -n 1

deps-linux-fedora: ## Linux: Fedora / RHEL packages (dnf; unavailable ones are skipped)
	sudo dnf install -y --skip-unavailable $(DNF_PACKAGES)
	@# Any ffmpeg will do (videos): keep an installed one (e.g. RPM Fusion's,
	@# which conflicts with Fedora's ffmpeg-free), else prefer RPM Fusion's.
	@if command -v ffmpeg >/dev/null 2>&1; then echo "ffmpeg already installed: $$(command -v ffmpeg)"; \
	elif dnf repolist --enabled 2>/dev/null | grep -qi rpmfusion-free; then sudo dnf install -y ffmpeg; \
	else sudo dnf install -y ffmpeg-free; fi

deps-linux-arch: ## Linux: Arch / Manjaro packages (pacman; add your GPU's Vulkan driver)
	sudo pacman -S --needed --noconfirm $(PACMAN_PACKAGES)

deps-linux-unknown:
	@echo "Unknown Linux distribution (/etc/os-release ID / ID_LIKE). Run one of" >&2
	@echo "  make deps-linux-debian | deps-linux-fedora | deps-linux-arch" >&2
	@echo "or install the equivalents of these Debian packages by hand:" >&2
	@echo "  $(APT_PACKAGES)" >&2
	@exit 1

deps-appimage: ## Linux: appimagetool to ~/.local/bin (no sudo; used by scripts/build-appimage.sh)
	@if [ -x "$(APPIMAGETOOL)" ]; then echo "appimagetool already installed: $(APPIMAGETOOL)"; exit 0; fi; \
	set -e; mkdir -p "$(dir $(APPIMAGETOOL))"; \
	echo "Downloading $(APPIMAGETOOL_URL)"; \
	curl --proto '=https' --tlsv1.2 -fsSL -o "$(APPIMAGETOOL).download" "$(APPIMAGETOOL_URL)"; \
	chmod +x "$(APPIMAGETOOL).download"; \
	if ! APPIMAGE_EXTRACT_AND_RUN=1 "$(APPIMAGETOOL).download" --version; then \
		echo "The downloaded appimagetool does not run: $(APPIMAGETOOL).download" >&2; exit 1; fi; \
	mv "$(APPIMAGETOOL).download" "$(APPIMAGETOOL)"; \
	echo "Installed $(APPIMAGETOOL)"

deps-windows: ## Windows: Git, VS Build Tools (C++), rustup via winget + Rust toolchain
	winget install --id Git.Git -e --accept-source-agreements --accept-package-agreements
	winget install --id Microsoft.VisualStudio.2022.BuildTools -e --accept-source-agreements --accept-package-agreements \
		--override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
	winget install --id Rustlang.Rustup -e --accept-source-agreements --accept-package-agreements
	rustup show active-toolchain || rustup toolchain install
	@echo "Open a new terminal so PATH includes rustup, Git and the MSVC tools."

deps-macos: ## macOS: Xcode Command Line Tools + ffmpeg (Homebrew; prints how to get it if missing) + Rust toolchain
	@xcode-select -p >/dev/null 2>&1 || { xcode-select --install; \
		echo "Finish the Command Line Tools installer, then run 'make deps-macos' again."; exit 1; }
	@ffmpeg=$$(command -v ffmpeg || ls /opt/homebrew/bin/ffmpeg /usr/local/bin/ffmpeg 2>/dev/null | head -n 1); \
		brew=$$(command -v brew || ls /opt/homebrew/bin/brew /usr/local/bin/brew 2>/dev/null | head -n 1); \
		if [ -n "$$ffmpeg" ]; then echo "ffmpeg found: $$ffmpeg"; \
		elif [ -n "$$brew" ]; then "$$brew" install ffmpeg || \
			echo "brew install ffmpeg failed (Homebrew owned by another user?): run it as the Homebrew admin user."; \
		else echo "Homebrew not found: skipping ffmpeg (only needed for video recording, key V)."; \
		echo "To record videos, install Homebrew (asks for an admin password), then ffmpeg:"; \
		echo '  /bin/bash -c "$$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"'; \
		echo '  brew install ffmpeg'; fi
	$(MAKE) deps-rust

deps-rust: ## Linux/macOS: rustup (if missing) + the pinned toolchain incl. wasm target
	@command -v rustup >/dev/null 2>&1 || \
		curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | \
		sh -s -- -y --no-modify-path --default-toolchain none
	rustup show active-toolchain || rustup toolchain install
	@echo "Toolchain ready. Add ~/.cargo/bin to PATH (e.g. '. \"\$$HOME/.cargo/env\"' in ~/.profile) to use cargo outside make."

build: ## Debug build of all crates and apps (with voice if clang-21 is installed)
	$(VOICE_ENV) $(CARGO) build --workspace $(VOICE_FEATURES)

release: ## Optimized build of ocw-desktop and ocw-server (with voice if clang-21 is installed)
	$(VOICE_ENV) $(CARGO) build --release -p ocw-desktop -p ocw-server $(VOICE_FEATURES)

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
