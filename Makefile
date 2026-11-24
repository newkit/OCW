# Open Cycling World – developer tasks (Linux).
# `make help` lists the targets.

# Use the rustup toolchain even if ~/.cargo/bin is not on PATH (Debian's
# system Rust is too old for Bevy 0.20).
export PATH := $(HOME)/.cargo/bin:$(PATH)
# On the build machines scripts/remote.sh sets OCW_CARGO_SHIM: its cargo
# (one build directory per feature set, spec §11.2) stays first.
ifneq ($(OCW_CARGO_SHIM),)
export PATH := $(OCW_CARGO_SHIM):$(PATH)
endif

CARGO ?= cargo
WASM_CRATES := -p ocw-core -p ocw-auth -p ocw-sim -p ocw-protocol -p ocw-world -p ocw-voice -p ocw-sky -p ocw-weather -p ocw-devices -p ocw-worldstore -p ocw-audio -p ocw-edit -p ocw-map
# The browser app (spec §14 Phase 10): wasm-bindgen-cli must be the version
# of the wasm-bindgen crate in Cargo.lock.
WASM_BINDGEN_VERSION := $(shell awk '/^name = "wasm-bindgen"$$/ { getline; gsub(/version = |"/, ""); print; exit }' Cargo.lock)
WEB_PORT ?= 8080
# make web-serve's address: 127.0.0.1, or 0.0.0.0 for the LAN (no secure context there: see the README).
WEB_BIND ?= 127.0.0.1
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
APT_PACKAGES := build-essential pkg-config curl git git-lfs rsync \
	libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev libxkbcommon-x11-0 \
	libdbus-1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
	mesa-vulkan-drivers vulkan-tools bluez \
	xvfb xauth weston ffmpeg libglib2.0-dev
# The same for Fedora / RHEL (dnf). ffmpeg is handled separately in
# deps-linux-fedora: Fedora's ffmpeg-free conflicts with RPM Fusion's ffmpeg.
DNF_PACKAGES := gcc gcc-c++ pkgconf-pkg-config curl git git-lfs rsync \
	alsa-lib-devel systemd-devel wayland-devel libxkbcommon-devel libxkbcommon-x11-devel \
	dbus-devel libX11-devel libXcursor-devel libXrandr-devel libXi-devel libxcb-devel \
	mesa-vulkan-drivers vulkan-tools bluez \
	xorg-x11-server-Xvfb xorg-x11-xauth weston fuse-libs glib2-devel
# The same for Arch / Manjaro (pacman); the Vulkan driver depends on the GPU
# (vulkan-radeon, vulkan-intel, nvidia-utils), mesa covers OpenGL.
PACMAN_PACKAGES := base-devel pkgconf curl git git-lfs rsync \
	alsa-lib systemd-libs wayland libxkbcommon libxkbcommon-x11 \
	dbus libx11 libxcursor libxrandr libxi libxcb \
	mesa vulkan-icd-loader vulkan-tools bluez bluez-utils \
	xorg-server-xvfb xorg-xauth weston ffmpeg fuse2 glib2
# VR (spec §14 Phase 11, 11D): the Khronos OpenXR loader and Monado, the
# open-source OpenXR runtime (BSL-1.0), whose simulated headset lets the VR
# tests run without hardware. Only for VR work and its tests; not part of
# `make deps`. Debian 13 names (marge, 2026-11-15: monado-service 25.0.0,
# libopenxr-loader1 1.1.47).
VR_APT_PACKAGES := libopenxr-loader1 libopenxr1-monado monado-service
VR_DNF_PACKAGES := openxr-libs monado
VR_PACMAN_PACKAGES := openxr monado
# The runtime manifest the loader picks when no other runtime is active.
MONADO_RUNTIME_JSON := /usr/share/openxr/1/openxr_monado.json
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
SCREENSHOT ?= /tmp/ocw.jpg
SCREENSHOT_WAYLAND ?= /tmp/ocw-wayland.jpg
VIDEO ?= /tmp/ocw.mp4
VIDEO_WAYLAND ?= /tmp/ocw-wayland.mp4

.DEFAULT_GOAL := help
.PHONY: help test-web-account test-web-trust version version-next version-bump test-release-version release-check release-prepare release-publish dashboard-install dashboard-uninstall art-gather art-sheets art-render art-diary art-publish art-collect art-check test-art art-install-timer art-uninstall-timer art-timer-status deps deps-linux deps-linux-packages deps-linux-debian deps-llvm-debian deps-linux-fedora deps-linux-arch deps-linux-unknown deps-appimage deps-blender test-blender models-diff deps-windows deps-macos deps-rust deps-sccache deps-sccache-windows build release lint fmt fmt-check \
	clippy check run server materials deploy-test clean clean-test-output macos-app \
	test test-unit test-wasm test-web-build test-web test-web-xr web-xr test-web-webgl2 test-web-gpu test-web-webkit test-web-ios deps-webkit test-web-world test-web-unicorn test-web-multiplayer test-web-ble test-web-ble-emulation test-web-ble-real test-web-audio test-web-voice test-web-livekit test-voice-livekit test-voice-bundled test-host-voice test-voice-live test-kiosk web web-serve web-worlds web-remote web-preview web-preview-site web-preview-worlds test-web-preview deps-web deps-browser deps-vr deps-vr-linux deps-vr-linux-debian deps-vr-linux-fedora deps-vr-linux-arch deps-vr-linux-unknown deps-vr-windows deps-vr-macos test-web-readme test-remote build-profile remote-gc remote-gc-branch janitor-plan windows-move-toolchain test-screenshot test-screenshot-wayland test-smoke test-video test-video-wayland \
	test-multiplayer test-multiplayer-local test-scenes test-scenes-list \
	wasm screenshot screenshot-wayland smoke video video-wayland

help: ## Show this help
	@grep -E '^[a-z0-9-]+:.*## ' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*## "}; {printf "  \033[36m%-23s\033[0m %s\n", $$1, $$2}'
	@echo "  Deprecated aliases (removed later): wasm, screenshot, screenshot-wayland, smoke, video, video-wayland"
	@echo "  -> test-wasm, test-screenshot, test-screenshot-wayland, test-smoke, test-video, test-video-wayland"

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

# Blender (spec 14 Phase 15, 15G Q4): the official release tarball, pinned by
# version and SHA-256 in scripts/deps-blender.sh (the one place), into
# ~/.local/opt/blender-X.Y.Z with ~/.local/bin/blender; for test-blender and
# the .blend sources on marge. A tool only: nothing of it is linked or shipped.
deps-blender: ## Linux x64: the pinned Blender LTS to ~/.local/opt/blender-X.Y.Z + ~/.local/bin/blender (SHA-256 checked, no sudo, idempotent; prints the version; spec 15G Q4)
	scripts/deps-blender.sh

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

deps-vr: deps-vr-$(PLATFORM) ## VR (Phase 11): the OpenXR loader and a runtime for this platform (Linux: Monado; sudo)

deps-vr-linux: deps-vr-linux-$(LINUX_FAMILY) ## VR on Linux: OpenXR loader + Monado (its simulated headset runs the VR tests without hardware; sudo)
	@command -v monado-service >/dev/null 2>&1 || { echo "monado-service not found after the install" >&2; exit 1; }
	@if [ -f $(MONADO_RUNTIME_JSON) ]; then echo "Monado runtime: $(MONADO_RUNTIME_JSON)"; \
	else echo "No $(MONADO_RUNTIME_JSON): set XR_RUNTIME_JSON to Monado's openxr_monado.json" >&2; fi
	@echo "Active OpenXR runtime: $$(readlink -f /etc/xdg/openxr/1/active_runtime.json 2>/dev/null || echo 'none set (the loader then needs XR_RUNTIME_JSON)')"

deps-vr-linux-debian:
	sudo apt-get install -y $(VR_APT_PACKAGES)

deps-vr-linux-fedora:
	sudo dnf install -y --skip-unavailable $(VR_DNF_PACKAGES)

deps-vr-linux-arch:
	sudo pacman -S --needed --noconfirm $(VR_PACMAN_PACKAGES)

deps-vr-linux-unknown:
	@echo "Unknown Linux distribution: install the OpenXR loader and Monado by hand (Debian names: $(VR_APT_PACKAGES))" >&2
	@exit 1

deps-vr-windows: ## VR on Windows: prints what to install (the Meta Quest Link app is not on winget)
	@echo "Install the Meta Quest Link app (https://www.meta.com/help/quest/pcvr/) and set it as the active OpenXR runtime;"
	@echo "the OpenXR loader comes with it. SteamVR is the alternative runtime."

deps-vr-macos: ## VR on macOS: none (no OpenXR runtime for macOS, spec §14 Phase 11)
	@echo "No VR on macOS: there is no OpenXR runtime for it (spec §14 Phase 11)."

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

# sccache (spec §11.2): a compiler cache on each build machine, shared by
# all its checkouts (every agent worktree has its own checkout and target/),
# so a new checkout takes the dependencies (Bevy, …) from the cache instead
# of compiling them. scripts/remote.sh uses it when ~/.local/bin/sccache
# exists (OCW_SCCACHE=0 turns it off). The release binary is pinned by
# version and SHA-256 (mozilla/sccache GitHub release, Apache-2.0).
SCCACHE_VERSION := 0.18.0
SCCACHE_BIN ?= $(HOME)/.local/bin/sccache
SCCACHE_SHA256_x86_64-unknown-linux-musl := 45f1447fbe231e3037bde351ef70677dd212216c8d62ae7ca409fecc4d6acc89
SCCACHE_SHA256_aarch64-apple-darwin := 308184519b646f5125289e8515b36f6ca65a13a041923994aebe702348674e8e
SCCACHE_SHA256_x86_64-pc-windows-msvc := 8965c74d5e8a225244f741e18ad2f3f504f48228dc1bac948fc22761a348363d

deps-sccache-windows: ## Windows build machine MACHINE (run from here): sccache $(SCCACHE_VERSION), MSVC build (pinned, SHA-256 checked) to %USERPROFILE%\.local\bin
	@[ -n "$(MACHINE)" ] || { echo "usage: make deps-sccache-windows MACHINE=<windows machine>" >&2; exit 2; }
	scripts/remote.sh $(MACHINE) run 'powershell -NoProfile -ExecutionPolicy Bypass -File scripts/install-sccache.ps1 -Version $(SCCACHE_VERSION) -Sha256 $(SCCACHE_SHA256_x86_64-pc-windows-msvc)'

deps-sccache: ## Linux x86_64 / macOS arm64: sccache $(SCCACHE_VERSION) (pinned, SHA-256 checked) to ~/.local/bin (no sudo; used by scripts/remote.sh)
	@set -e; case "$$(uname -s)-$$(uname -m)" in \
		Linux-x86_64) t=x86_64-unknown-linux-musl; sum=$(SCCACHE_SHA256_x86_64-unknown-linux-musl) ;; \
		Darwin-arm64) t=aarch64-apple-darwin; sum=$(SCCACHE_SHA256_aarch64-apple-darwin) ;; \
		*) echo "no pinned sccache build for $$(uname -sm)" >&2; exit 1 ;; esac; \
	if "$(SCCACHE_BIN)" --version 2>/dev/null | grep -qx "sccache $(SCCACHE_VERSION)"; then \
		echo "sccache $(SCCACHE_VERSION) already installed: $(SCCACHE_BIN)"; exit 0; fi; \
	tmp=$$(mktemp -d); trap 'rm -rf "$$tmp"' EXIT; name=sccache-v$(SCCACHE_VERSION)-$$t; \
	curl --proto '=https' --tlsv1.2 -fsSL -o "$$tmp/$$name.tar.gz" \
		"https://github.com/mozilla/sccache/releases/download/v$(SCCACHE_VERSION)/$$name.tar.gz"; \
	got=$$( (sha256sum "$$tmp/$$name.tar.gz" 2>/dev/null || shasum -a 256 "$$tmp/$$name.tar.gz") | cut -d' ' -f1); \
	[ "$$got" = "$$sum" ] || { echo "sccache download: SHA-256 $$got, expected $$sum" >&2; exit 1; }; \
	tar -xzf "$$tmp/$$name.tar.gz" -C "$$tmp"; \
	mkdir -p "$(dir $(SCCACHE_BIN))"; install -m 755 "$$tmp/$$name/sccache" "$(SCCACHE_BIN)"; \
	"$(SCCACHE_BIN)" --version

build: ## Debug build of all crates and apps (with voice if clang-21 is installed)
	$(VOICE_ENV) $(CARGO) build --workspace $(VOICE_FEATURES)

release: ## Optimized build of ocw-desktop and ocw-server (with voice if clang-21 is installed)
	$(VOICE_ENV) $(CARGO) build --release -p ocw-desktop -p ocw-server $(VOICE_FEATURES)

# Tests are the test-* targets (spec §11.4): test-unit, test-wasm and
# test-kiosk need no display, GPU or other machine; test-screenshot*,
# test-smoke and test-video* run the desktop app headlessly (Linux);
# test-multiplayer uses the build machines of config/machines.toml.
test: test-unit ## All unit and integration tests (= test-unit; no hardware, network or GPU)

test-unit: ## Run all unit and integration tests (cargo test --workspace)
	$(CARGO) test --workspace

fmt: ## Format all code
	$(CARGO) fmt --all

fmt-check: ## Check formatting
	$(CARGO) fmt --all -- --check

clippy: ## Lint; warnings are errors (as in CI)
	$(CARGO) clippy --workspace --all-targets -- -D warnings

lint: fmt-check clippy ## fmt-check + clippy

test-wasm: ## Check that the core crates (and the server lib) build for wasm32-unknown-unknown (spec §3.5)
	$(CARGO) check $(WASM_CRATES) --target wasm32-unknown-unknown
	$(CARGO) check -p ocw-server --lib --target wasm32-unknown-unknown

test-web-build: ## Build the browser app for wasm32, the WebGPU, the WebGL 2 and the WebXR build (spec §14 Phase 10, B7; Phase 14 14I; CI: part of check)
	$(CARGO) build -p ocw-web --target wasm32-unknown-unknown --features webgpu
	$(CARGO) build -p ocw-web --target wasm32-unknown-unknown --features webgl2
	$(CARGO) build -p ocw-web --target wasm32-unknown-unknown --features webgl2,webxr

deps-web: ## The browser app's tools: wasm-bindgen-cli matching Cargo.lock (cargo install, no sudo)
	@if [ "$$(wasm-bindgen --version 2>/dev/null)" = "wasm-bindgen $(WASM_BINDGEN_VERSION)" ]; then \
		echo "wasm-bindgen $(WASM_BINDGEN_VERSION) installed"; \
	else $(CARGO) install wasm-bindgen-cli --version $(WASM_BINDGEN_VERSION) --locked; fi

web: ## Build the browser app into target/web/ (the `web-release` profile; WEB_PROFILE=dev for a debug build)
	scripts/build-web.sh

web-xr: ## The browser app with its WebXR build too (spec §14 Phase 14, 14I W1): target/web/ with ocw_web_webxr.js (WebGL 2 + feature webxr, the page's ?xr=1)
	WEB_GPU="webgpu webgl2 webxr" scripts/build-web.sh

web-serve: ## Serve target/web/ (and its worlds/, with range requests) on http://$(WEB_BIND):$(WEB_PORT)/ (COOP/COEP headers; WEB_PORT, WEB_BIND=0.0.0.0 for the LAN); fails without a build
	scripts/web-serve.py --port $(WEB_PORT) --bind $(WEB_BIND)

web-remote: ## Build the browser app on build machine MACHINE (make web; WORLDS=1: also make web-worlds) and fetch the page into target/web/ here (no cargo output, no private keys)
	@[ -n "$(MACHINE)" ] || { echo "usage: make web-remote MACHINE=<build machine> [WORLDS=1]" >&2; exit 2; }
	WORLDS=$(WORLDS) scripts/web-remote.sh $(MACHINE)

deps-browser: ## The browser tests' browser: pinned Chromium (scripts/browsers.json, SHA-256 checked) into ~/.cache/ocw/browsers/ (no sudo; Linux x64, macOS arm64, Windows x64)
	python3 scripts/web_browser.py fetch

PLAYWRIGHT_VENV ?= $(HOME)/.cache/ocw/playwright-venv
PLAYWRIGHT_VERSION ?= 1.65.2
IOS_MINUTES ?= 5
deps-webkit: ## Safari's engine for test-web-webkit: Playwright $(PLAYWRIGHT_VERSION) and its WebKit build in a venv ($(PLAYWRIGHT_VENV); no sudo; Linux needs WebKit's libraries, which Debian 13 has)
	python3 -m venv $(PLAYWRIGHT_VENV)
	$(PLAYWRIGHT_VENV)/bin/pip install -q playwright==$(PLAYWRIGHT_VERSION)
	$(PLAYWRIGHT_VENV)/bin/python -m playwright install webkit

# The pilot package (git-ignored data/): a branch checkout of scripts/remote.sh
# (REPO_PATH-BRANCH) finds it in the main checkout next to it.
WEB_WORLD_PKGS ?= $(firstword $(wildcard data/pilot/liechtenstein.ocwpkg ../OCW/data/pilot/liechtenstein.ocwpkg) data/pilot/liechtenstein.ocwpkg)
web-worlds: ## Publish world packages for the web dev server into target/web/worlds/, signed by a dev root in target/web-keys/ (WEB_WORLD_PKGS)
	scripts/publish-web-worlds.sh $(WEB_WORLD_PKGS)

web-preview: ## The public preview's build into target/web-preview/ (spec §14 Phase 10 B8a): fat LTO, wasm-opt -Oz (pinned binaryen), only the web preview's signing root trusted, no LiveKit
	scripts/build-web.sh --preview

web-preview-site: ## Assemble the preview's site into target/web-preview-site/ (config.json, credits.html, the signed worlds of target/web-preview-worlds/release/ or a downloaded web-preview-worlds release; PREVIEW_WORLDS)
	scripts/web-preview-site.py --worlds $(PREVIEW_WORLDS)

web-preview-worlds: ## On marge only (the preview's keys): sign the preview's worlds, publish them and upload them to the pre-release web-preview-worlds
	scripts/publish-preview-worlds.sh --upload

test-web-preview: ## Headless Chromium on the preview: the assembled site (PREVIEW_SITE, served as a public page) or the live one (PREVIEW_URL=https://newkit.github.io/OCW/); start page, Hills, Liechtenstein from the preview root, dev trust ignored, WebGL 2, credits
	scripts/test-web-preview.py $(if $(PREVIEW_URL),--url $(PREVIEW_URL),--site $(PREVIEW_SITE)) --out test-output/web-preview

test-web: web ## Headless Chromium on WebGPU: load the web app, ride, screenshot into test-output/web/
	scripts/test-web.py --out test-output/web

test-web-account: web ## Headless Chromium: sign in with the fake Google through the redirect, link Strava, sign out (ocw-server --auth-dev, spec §9.1); screenshots into test-output/web/
	$(CARGO) build -p ocw-server
	scripts/test-web-account.py --out test-output/web

test-web-xr: web-xr ## Headless Chromium with the Immersive Web Emulation Runtime (iwer, Quest 1 profile): Enter VR, a stereo frame of the test route, head tracking, the copy into a framebuffer, exit (14I W1); eyes and the emulated view into test-output/web-xr/
	scripts/test-web-xr.py --out test-output/web-xr

test-web-webgl2: web ## Headless Chromium without WebGPU: the page falls back to the WebGL 2 build (ride, menus, fps); then WebGPU High vs WebGL 2 Low, same view, and the Graphics page (B7); the GPU choice and the WebGPU fallback (test-web-gpu); into test-output/web/
	scripts/test-web.py --out test-output/web --gpu webgl2
	scripts/test-web-webgl2.py --out test-output/web
	scripts/test-web-gpu.py --out test-output/web

test-web-gpu: web ## Headless Chromium: the page's GPU choice (ocw-gpu.js rules), injected WebGPU failures fall back to WebGL 2 and draw, the remembered failure, gpu=webgpu, Safari on an iPhone takes WebGL 2 (B7, the iOS 26 report); into test-output/web/
	scripts/test-web-gpu.py --out test-output/web

test-web-webkit: web ## WebKit (Safari's engine, Playwright; make deps-webkit): Safari 26 with WebGPU takes the WebGL 2 build – an iPhone 15's start page and ride (portrait and landscape: the safe areas, touch taps and a held side, the manifest, the wake lock; spec 3.2.3 TS4; the phone layout and the touch power buttons, spec 3.2, TS5), Safari on macOS with the desktop HUD unchanged; rides and draws (B7); into test-output/web/
	scripts/test-web-webkit.py --out test-output/web

test-web-ios: web ## WebKit iPhone 15 (Playwright; make deps-webkit): a long Hills ride (IOS_MINUTES, 5) with the page hidden and shown every minute, then the WebGL context lost while hidden (the app stops, the ride loads again once) and again (a message, no second reload) – the iPhone restarts of 2026-11-22 (spec 14 Phase 10 B7); memory every 5 s (wasm + all WebGL memory, render targets included) against the phone budget of v5.42 (test routes 200 MB, worlds 450 MB, peak and end of the ride); worlds: scripts/test-web-ios.py --world ID --route NAME --deaths; into test-output/web-ios/
	scripts/test-web-ios.py --out test-output/web-ios --minutes $(IOS_MINUTES)

PREVIEW_WORLDS ?= target/web-preview-worlds/release
PREVIEW_SITE ?= target/web-preview-site
PREVIEW_URL ?=
WEB_WORLD ?= liechtenstein
WEB_WORLD_ROUTE ?= vaduz-malbun
test-web-world: web web-worlds ## Headless Chromium: stream $(WEB_WORLD) from the dev server, ride $(WEB_WORLD_ROUTE), teleport, reload from the OPFS cache; screenshots into test-output/web/
	scripts/test-web.py --out test-output/web --world $(WEB_WORLD) --route $(WEB_WORLD_ROUTE) --seconds 15 --teleport 6 --time noon

test-web-trust: web ## Headless Chromium: a world signed by a throwaway user key opens the trust dialog (spec §4.7 User keys): fingerprint, Cancel focused, declined, trusted, remembered; screenshots into test-output/web/
	scripts/test-web-trust.py --out test-output/web --package $(firstword $(WEB_WORLD_PKGS))

test-web-unicorn: web web-worlds ## Headless Chromium: unicorn mode over $(WEB_WORLD) from $(WEB_WORLD_ROUTE)'s start (spec §14 Phase 13 G5): G, +, inspection, a pin, the menu row, landing; screenshots into test-output/web/
	scripts/test-web.py --out test-output/web --world $(WEB_WORLD) --route $(WEB_WORLD_ROUTE) --seconds 8 --no-reload --god --time noon

test-web-multiplayer: web ## Headless Chromium joins ocw-server over WebTransport with ocw-bot riders (QUIC + WebTransport), rides 30 s; screenshot, timing into test-output/web/
	$(CARGO) build --release -p ocw-server -p ocw-bot
	scripts/test-web-multiplayer.py --out test-output/web --seconds 30

test-web-ble: web ## Headless Chromium with a fake Web Bluetooth FTMS trainer + HR strap: connect by click, power from the device, SIM / ERG writes, drop + reconnect, reload, no-Bluetooth fallback; screenshots into test-output/web/
	scripts/test-web-ble.py --out test-output/web --mode fake

test-web-ble-emulation: web ## Headless Chromium's own Web Bluetooth with a simulated KICKR (DevTools BluetoothEmulation + DeviceAccess): chooser, connect, discovery, Wahoo control writes
	scripts/test-web-ble.py --out test-output/web --mode emulation

BLE_DEVICE ?= KICKR
test-web-ble-real: web ## Headless Chromium rides the real trainer in range ($(BLE_DEVICE), marge's KICKR) over BlueZ: the chooser answered over DevTools, SIM, ERG
	scripts/test-web-ble.py --out test-output/web --mode real --device $(BLE_DEVICE) --seconds 20

BRIDGE_DEVICES ?=
BRIDGE_SECONDS ?= 120
test-bridge: web web-worlds ## The sensor bridge end to end (spec 14 Phase 10, 10D.2): the desktop shares its devices ($(BRIDGE_DEVICES): a devices file, e.g. marge's KICKR; default the virtual trainer), headless Chromium pairs by the code and rides $(WEB_WORLD_ROUTE) $(BRIDGE_SECONDS) s; latency, control, screenshots into test-output/bridge/
	scripts/test-bridge.py --out test-output/bridge --seconds $(BRIDGE_SECONDS) --world $(WEB_WORLD) --route $(WEB_WORLD_ROUTE) $(if $(BRIDGE_DEVICES),--devices-file $(BRIDGE_DEVICES))

test-web-audio: web ## Headless Chromium: the game sound locked until a click (autoplay rule), then the mixer's chunks scheduled through Web Audio with sound in them; a volume kept over a reload; screenshots into test-output/web/
	scripts/test-web-audio.py --out test-output/web --part audio

test-web-voice: web ## Headless Chromium joins ocw-server (voice tokens, Echo rider) over WebTransport with an ocw-bot; a fake LiveKit in the page: token, call, M / Shift+M, speakers, drop, rejoin, echo test on the fake microphone, reload
	$(CARGO) build --release -p ocw-server -p ocw-bot
	scripts/test-web-audio.py --out test-output/web --part voice

LIVEKIT_SERVER ?= livekit-server
test-web-livekit: web ## Three headless browser riders in a real `livekit-server --dev` room (the real livekit-client; skipped without the binary): join (muted, heard by no one), see and hear each other, mute, a block both ways (spec §7.1.7 Q9), range (a far rider not sent nor heard, refused) and Who hears me / Voice room (§7.1.7 Q6, Q8)
	$(CARGO) build --release -p ocw-server -p ocw-bot
	scripts/test-web-audio.py --out test-output/web --part livekit --livekit-server $(LIVEKIT_SERVER)

test-voice-livekit: ## The LiveKit Rust backend against its own livekit-server on free loopback ports: two riders and two desktop VoiceControllers see and hear each other, mute, leave; three controllers: a block holds both ways (§7.1.7 Q9), the game server's range and Who hears me hold (§7.1.7 Q6, Q8) (spec §7.1.8; skipped without the binary; needs clang-21 on Linux)
	CC=clang-21 CXX=clang++-21 OCW_LIVEKIT_SERVER=$(if $(filter livekit-server,$(LIVEKIT_SERVER)),,$(LIVEKIT_SERVER)) $(CARGO) test -p ocw-voice --features livekit --test livekit_two_participants --test backend_events -- --nocapture

test-voice-bundled: ## `ocw-server --listen` starts the bundled livekit-server by itself (secret only in its environment); two headless riders get grants and hear each other; three riders: a block holds both ways (§7.1.7 Q9); range and Who hears me over QUIC (§7.1.7 Q6, Q8); SIGKILL of the server ends it (spec §7.1, V1; skipped without the binary)
	scripts/test-voice-bundled.sh $(LIVEKIT_SERVER)

test-host-voice: ## Ride -> Host a ride (headless app) starts the bundled livekit-server through the shared launcher; two headless riders get grants and join the room; it ends with the app (spec §7.1, V2; skipped without the binary)
	scripts/test-host-voice.sh $(LIVEKIT_SERVER)

test-voice-live: ## The desktop app (voice build, headless) joins an `ocw-server` with its bundled livekit-server as a rider: grant, room, microphone published, heard by the echo participant (spec §7.1.8; skipped without the binary; needs clang-21 on Linux)
	scripts/test-voice-live.sh $(LIVEKIT_SERVER)

test-web-readme: deps-browser ## The README's browser walkthrough in a fresh clone: deps-web, web, web-worlds, web-serve, every README URL in the pinned browser; screenshots + verdict into test-output/web/readme-<time>/ (README_FLAGS="--keep --reuse-target": quicker reruns)
	scripts/test-web-readme.py --out test-output/web $(README_FLAGS)

test-kiosk: ## Test the kiosk script with a fake app (no display or build needed)
	scripts/test-kiosk.sh

# The model files' tools (spec 14 Phase 15, 15D / 15F). MODELS limits the
# round trip to manifest patterns (MODELS="bikes/* wear/helmet-*").
MODELS ?=
test-blender: ## Blender round trip of every model file (spec 15F): Blender imports and re-exports it with the README's preset, `model import` takes it back, `model diff` must be empty; summary into test-output/blender-roundtrip/ (skipped without Blender: make deps-blender, on marge); also the deps-blender and models-diff script tests
	scripts/test-deps-blender.sh
	CARGO="$(CARGO)" scripts/test-models-diff.sh
	CARGO="$(CARGO)" scripts/blender-roundtrip.py $(if $(MODELS),--only $(MODELS))

BASE ?= main
models-diff: ## The structural diff (`ocw-assets model diff`) of the model files changed against BASE (default main; committed, uncommitted and new files, the manifest); MACHINE=marge: git here, the diff there (spec 15D)
	@if [ -n "$(MACHINE)" ]; then \
		set -e; d=target/models-diff; rm -rf $$d; \
		scripts/models-diff.sh --base $(BASE) --stage $$d/stage >/dev/null; \
		tar -cf $$d/stage.tar -C $$d/stage .; \
		scripts/remote.sh $(MACHINE) put $$d/stage.tar target/models-diff; \
		scripts/remote.sh $(MACHINE) run 'scripts/models-diff.sh --staged target/models-diff/stage.tar'; \
	else CARGO="$(CARGO)" scripts/models-diff.sh --base $(BASE); fi

remote-gc: ## Build machines: list the target/ of merged or gone agent checkouts and their size (GC=1 deletes them; MACHINES="a b" limits)
	GC=$(GC) scripts/remote-gc.sh $(MACHINES)

remote-gc-branch: ## Cleanup on merge: delete the target/ of BRANCH's checkout on every reachable build machine (DRY=1: only list)
	@[ -n "$(BRANCH)" ] || { echo "usage: make remote-gc-branch BRANCH=<branch> [DRY=1]" >&2; exit 2; }
	scripts/remote-gc.sh --branch $(BRANCH) $(if $(DRY),--dry-run) $(MACHINES)

janitor-plan: ## Dry run of the dashboard's disk janitor on every reachable machine: what each threshold would delete
	python3 tools/build-dashboard/dashboard.py --janitor-plan --no-record

windows-move-toolchain: ## Windows MACHINE: dry run of moving CARGO_HOME, RUSTUP_HOME, sccache from C: to B:\toolchain (APPLY=1 moves)
	@[ -n "$(MACHINE)" ] || { echo "usage: make windows-move-toolchain MACHINE=<windows machine> [APPLY=1]" >&2; exit 2; }
	scripts/remote.sh $(MACHINE) run 'powershell -NoProfile -ExecutionPolicy Bypass -File scripts/windows-move-toolchain.ps1 $(if $(APPLY),-Apply)'

test-remote: ## Test remote.sh's build-slot order, tree snapshot and sync, the build-log parser and the web scripts (web-serve, web_browser, the README walkthrough's parser; no machine needed)
	scripts/test-remote.sh

# Build profile (spec §11.2): clean, no-op and incremental builds of the main
# cargo commands on MACHINE, with cargo's --timings reports, fetched into
# test-output/build-profile/. Takes about 45 min on marge (one build slot).
BUILD_PROFILE_LABEL ?= $(or $(MACHINE),marge)-$(shell date +%Y%m%d-%H%M)
build-profile: ## Profile clean/no-op/incremental builds on MACHINE (default marge) into test-output/build-profile/
	scripts/remote.sh $(or $(MACHINE),marge) run 'scripts/build-profile.sh --label $(BUILD_PROFILE_LABEL)'
	scripts/remote.sh $(or $(MACHINE),marge) fetch test-output/build-profile test-output

check: lint test-unit test-wasm test-web-build ## fmt, clippy, tests, wasm, web build (CI also runs the smoke tests)

run: ## Start the desktop client (needs a display)
	$(CARGO) run -p ocw-desktop

server: ## Run the headless server for 10 s with 3 test riders
	$(CARGO) run -p ocw-server -- --bots 3 --seconds 10 --fast

test-screenshot: ## Headless smoke test on X11 (Xvfb); writes $(SCREENSHOT)
	.claude/skills/run-ocw/drive.sh shot $(SCREENSHOT)

test-screenshot-wayland: ## Headless smoke test on Wayland (weston); writes $(SCREENSHOT_WAYLAND)
	.claude/skills/run-ocw/drive.sh shot-wayland $(SCREENSHOT_WAYLAND)

test-smoke: test-screenshot test-screenshot-wayland ## Both desktop smoke tests (X11 and Wayland)

test-video: ## Record a smooth 6 s test ride (X11, frame capture); writes $(VIDEO)
	.claude/skills/run-ocw/drive.sh video $(VIDEO)

test-video-wayland: ## Same on Wayland (weston); writes $(VIDEO_WAYLAND)
	.claude/skills/run-ocw/drive.sh video-wayland $(VIDEO_WAYLAND)

test-scenes: ## Render scripts/scenes.toml headlessly (Wayland) into test-output/scenes/ (ONLY=glob,..., WORLD=pkg, REMOTE=machine)
	scripts/render-scenes.sh $(if $(ONLY),--only '$(ONLY)') $(if $(WORLD),--world '$(WORLD)') $(if $(REMOTE),--remote $(REMOTE))

test-scenes-list: ## List the scenes of scripts/scenes.toml
	scripts/render-scenes.sh --list
test-multiplayer: ## Server + kiosk on marge, ocw-bot riders on the other build machines over QUIC (spec §11.4.1)
	scripts/test-multiplayer.sh

MP_LOCAL := target/mp-local
test-multiplayer-local: ## ocw-server + 3 ocw-bot riders on this machine (loopback QUIC) and the verdict
	$(CARGO) build -p ocw-server -p ocw-bot
	@set -e; mkdir -p $(MP_LOCAL); rm -f $(MP_LOCAL)/*.log $(MP_LOCAL)/stats.json; \
	target/debug/ocw-server --listen 127.0.0.1:47899 --bots 1 --seconds 25 \
		--stats-file $(MP_LOCAL)/stats.json >$(MP_LOCAL)/server.log 2>&1 & server=$$!; \
	sleep 2; \
	target/debug/ocw-bot --server 127.0.0.1:47899 --insecure-dev --riders 3 --seconds 0 \
		>$(MP_LOCAL)/bot.log 2>&1 || true; \
	wait $$server; \
	target/debug/ocw-bot verdict --server-stats $(MP_LOCAL)/stats.json --processes 1 \
		--riders-per-process 3 $(MP_LOCAL)/bot.log

# Deprecated names of the test targets (kept for a while; not in `help`).
wasm screenshot screenshot-wayland smoke video video-wayland:
	@echo "make $@ is deprecated: use make test-$@" >&2
	@$(MAKE) --no-print-directory test-$@

materials: ## Fetch the material library (pinned CC0 textures → KTX2) into assets/materials/ (spec §4.6)
	scripts/fetch-materials.sh

deploy-test: ## Build on MACHINE (default marge) and deploy for its kiosk (scripts/ocw-kiosk.sh)
	scripts/deploy-test.sh $(or $(MACHINE),marge)

# macOS app bundle (spec §11.3).
macos-app: ## macOS: Open Cycling World.app + DMG + zip in target/macos (ad-hoc signed)
	scripts/build-macos-app.sh

clean: ## Remove build artefacts
	$(CARGO) clean

clean-test-output: ## Delete the archived screenshots and videos in test-output/
	rm -rf test-output

# Art project: the OCW Build Diary moved to its own repository on 2026-11-15,
# https://github.com/newkit/OCW-ART (a checkout next to this one, ../OCW-ART).
# The old targets only say where they went; art-uninstall-timer still works,
# to stop a collector timer that was installed from this checkout.
ART_MOVED = gather sheets render diary publish collect check install-timer timer-status
OCW_ART ?= $(abspath $(CURDIR)/../OCW-ART)
USER_UNITS = $(HOME)/.config/systemd/user

$(addprefix art-,$(ART_MOVED)) test-art:
	@t='$@'; t=$${t#art-}; [ "$$t" != test-art ] || t=test; \
	  echo "The art project moved to https://github.com/newkit/OCW-ART: run 'make $$t' in its checkout ($(OCW_ART))." >&2; exit 1

art-uninstall-timer: ## Art: stop and remove the art collector's systemd user timer (the collector lives in OCW-ART now)
	-systemctl --user disable --now ocw-art-collect.timer
	-systemctl --user stop ocw-art-collect.service
	rm -f $(USER_UNITS)/ocw-art-collect.service $(USER_UNITS)/ocw-art-collect.timer
	systemctl --user daemon-reload
	@echo "Removed. Install it from OCW-ART: make -C $(OCW_ART) install-timer"

dashboard-install: ## Install and start the build dashboard as a systemd user service (port 8765, starts at login) and the 15-minute timer that keeps its open questions current
	mkdir -p $(HOME)/.config/systemd/user
	sed 's|@REPO@|$(CURDIR)|g' tools/build-dashboard/ocw-build-dashboard.service > $(HOME)/.config/systemd/user/ocw-build-dashboard.service
	systemctl --user daemon-reload
	systemctl --user enable --now ocw-build-dashboard.service
	systemctl --user restart ocw-build-dashboard.service
	sed 's|@REPO@|$(CURDIR)|g' tools/build-dashboard/ocw-spec-questions.service > $(HOME)/.config/systemd/user/ocw-spec-questions.service
	cp tools/build-dashboard/ocw-spec-questions.timer $(HOME)/.config/systemd/user/ocw-spec-questions.timer
	systemctl --user daemon-reload
	systemctl --user enable --now ocw-spec-questions.timer
	@echo "Dashboard: http://localhost:8765/  (start at boot without login: sudo loginctl enable-linger $$USER)"

dashboard-uninstall: ## Stop and remove the build dashboard user service
	-systemctl --user disable --now ocw-build-dashboard.service
	-systemctl --user disable --now ocw-spec-questions.timer
	rm -f $(HOME)/.config/systemd/user/ocw-build-dashboard.service $(HOME)/.config/systemd/user/ocw-spec-questions.service $(HOME)/.config/systemd/user/ocw-spec-questions.timer
	systemctl --user daemon-reload

# --- Versions and GitHub releases (spec §11.3) ------------------------------
# Every GitHub release increments the patch version (0.2.0 -> 0.2.1,
# spec §11.3 v2.49): `make release-prepare`
# bumps [workspace.package] version in Cargo.toml (inherited by every crate)
# and the workspace crates in Cargo.lock, commits "Release X.Y.Z" and pushes;
# build the release binaries from that commit; `make release-publish` tags
# vX.Y.Z and creates the GitHub release. VERSION_ROOT points the version
# targets at another checkout (tests). Release tags: vX.Y.Z.
VERSION_ROOT ?= $(CURDIR)
PART ?= patch
GH_REPO ?= newkit/OCW
version_of = sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\([0-9.]*\)".*/\1/p' $(1)/Cargo.toml | head -1
# Highest X.Y.Z of the GitHub release tags (v0.1.0-abc counts as 0.1.0).
latest_release = gh release list -R $(GH_REPO) --limit 200 --json tagName -q '.[].tagName' 2>/dev/null | sed -n 's/^v\([0-9]*\.[0-9]*\.[0-9]*\).*/\1/p' | sort -t. -k1,1n -k2,2n -k3,3n | tail -1
# Shell function n A B: is A a higher X.Y.Z than B?
newer = n() { [ "$$1" != "$$2" ] && [ "$$(printf '%s\n%s\n' "$$1" "$$2" | sort -t. -k1,1n -k2,2n -k3,3n | tail -1)" = "$$1" ]; }

version: ## Print the workspace version
	@$(call version_of,$(VERSION_ROOT))

version-next: ## Print the version after a bump (PART=patch|minor|major, default patch)
	@cur=$$($(call version_of,$(VERSION_ROOT))); a=$${cur%%.*}; r=$${cur#*.}; b=$${r%%.*}; c=$${r#*.}; \
	case "$(PART)" in major) echo "$$((a+1)).0.0";; minor) echo "$$a.$$((b+1)).0";; patch) echo "$$a.$$b.$$((c+1))";; *) echo "PART: major|minor|patch" >&2; exit 2;; esac

version-bump: ## Bump the workspace version in Cargo.toml and Cargo.lock (PART=patch|minor|major, default patch)
	@set -e; root='$(VERSION_ROOT)'; cur=$$($(call version_of,$$root)); \
	nv=$$($(MAKE) -s --no-print-directory version-next VERSION_ROOT="$$root" PART=$(PART)); \
	members=$$(for m in $$(find "$$root/apps" "$$root/crates" "$$root/services" "$$root/tools" -name Cargo.toml -not -path '*/target/*' 2>/dev/null); do \
	  if grep -q '^version\.workspace *= *true' "$$m"; then sed -n 's/^name *= *"\([^"]*\)".*/\1/p' "$$m" | head -1; fi; done); \
	awk -v cur="$$cur" -v nv="$$nv" '/^\[/ { sec = $$0 } sec == "[workspace.package]" && $$0 ~ "^version *= *\"" cur "\"" { sub(cur, nv) } { print }' \
	  "$$root/Cargo.toml" > "$$root/Cargo.toml.tmp" && mv "$$root/Cargo.toml.tmp" "$$root/Cargo.toml"; \
	if [ -f "$$root/Cargo.lock" ]; then \
	  awk -v cur="$$cur" -v nv="$$nv" -v list="$$members" 'BEGIN { n = split(list, a, "\n"); for (i = 1; i <= n; i++) ws[a[i]] = 1 } \
	    /^name = "/ { name = $$0; sub(/^name = "/, "", name); sub(/".*/, "", name) } \
	    /^version = "/ && (name in ws) && $$0 == "version = \"" cur "\"" { $$0 = "version = \"" nv "\"" } { print }' \
	    "$$root/Cargo.lock" > "$$root/Cargo.lock.tmp" && mv "$$root/Cargo.lock.tmp" "$$root/Cargo.lock"; \
	fi; \
	[ "$$($(call version_of,$$root))" = "$$nv" ] || { echo "version bump failed" >&2; exit 1; }; echo "$$nv"

test-release-version: ## Test the version bump of GitHub releases on a copy of the manifests
	scripts/test-release-version.sh

release-check: ## Show the version, the latest GitHub release and the next release version
	@cur=$$($(call version_of,.)); last=$$($(latest_release)); $(newer); \
	echo "Cargo.toml version: $$cur"; echo "latest release:     $${last:-none}"; \
	if [ -n "$$last" ] && ! n "$$cur" "$$last"; then echo "next release:       $$($(MAKE) -s --no-print-directory version-next) (make release-prepare)"; \
	else echo "next release:       $$cur (prepared: build it, then make release-publish DIR=... NOTES=...)"; fi

release-prepare: ## Bump the patch version for the next GitHub release, commit "Release X.Y.Z" and push (then build from that commit)
	@set -e; [ "$$(git rev-parse --abbrev-ref HEAD)" = main ] || { echo "not on main" >&2; exit 1; }; \
	[ -z "$$(git status --porcelain --untracked-files=no)" ] || { echo "uncommitted changes" >&2; exit 1; }; \
	git fetch -q origin main; [ "$$(git rev-parse HEAD)" = "$$(git rev-parse origin/main)" ] || { echo "main is not origin/main" >&2; exit 1; }; \
	cur=$$($(call version_of,.)); last=$$($(latest_release)); $(newer); \
	if [ -n "$$last" ] && n "$$cur" "$$last"; then echo "version $$cur is already above the latest release $$last"; \
	else nv=$$($(MAKE) -s --no-print-directory version-bump PART=patch); git add Cargo.toml Cargo.lock; \
	  git commit -q -m "Release $$nv" -m "Bumps the workspace version for the GitHub release v$$nv (every release increments the patch version, spec §11.3)."; \
	  git push -q origin main; cur=$$nv; fi; \
	echo "version: $$cur"; echo "commit:  $$(git rev-parse --short HEAD)  (build the release binaries from this commit)"

release-publish: ## Tag vX.Y.Z and create the GitHub release (pre-release): make release-publish DIR=assets NOTES=notes.md
	@set -e; [ -d "$(DIR)" ] && [ -f "$(NOTES)" ] || { echo "usage: make release-publish DIR=... NOTES=..." >&2; exit 2; }; \
	cur=$$($(call version_of,.)); last=$$($(latest_release)); $(newer); \
	if [ -n "$$last" ] && ! n "$$cur" "$$last"; then echo "version $$cur is not above the latest release $$last: make release-prepare first" >&2; exit 1; fi; \
	tag="v$$cur"; commit=$$(git rev-parse HEAD); \
	if git rev-parse -q --verify "refs/tags/$$tag" >/dev/null; then commit=$$(git rev-list -n1 "$$tag"); \
	else git tag -a "$$tag" -m "Open Cycling World $$cur" "$$commit"; git push -q origin "$$tag"; fi; \
	(cd "$(DIR)" && sha256sum -- $$(ls | grep -v '^SHA256SUMS$$') > SHA256SUMS); \
	gh release create "$$tag" -R $(GH_REPO) --target "$$commit" --prerelease --title "Open Cycling World $$cur (development build)" --notes-file "$(NOTES)" "$(DIR)"/*
