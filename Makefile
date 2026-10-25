# Setup and builds for every part of the project. `make` lists the targets.
#
#   make setup         # toolchains and dependencies (once, and after they change)
#   make cli           # target/release/pdfi, with the web UI embedded
#   make web           # apps/web/dist (browser-only mode)
#   make macos         # the macOS app (CONFIGURATION=Debug for a debug build)
#   make check         # what CI runs

.DEFAULT_GOAL := help
.PHONY: help setup setup-rust setup-web all cli web wasm web-dev core core-debug \
	macos macos-open fixtures test test-web test-macos lint fmt deny check clean

WEB := apps/web
MACOS := apps/macos
# Must match crates/pdfi-wasm/Cargo.toml (scripts/build-wasm.sh checks it).
WASM_BINDGEN_VERSION := $(shell sed -n 's/^wasm-bindgen = "=\(.*\)"/\1/p' crates/pdfi-wasm/Cargo.toml)
CONFIGURATION ?= Release
DERIVED := $(MACOS)/build/DerivedData
# Core crates must stay buildable for the browser (CI checks the same list).
WASM_CRATES := -p pdfi-syntax -p pdfi-schema -p pdfi-model -p pdfi-font -p pdfi-content \
	-p pdfi-render -p pdfi-api -p pdfi-wasm

help: ## Show this list
	@awk 'BEGIN {FS = ":.*## "} /^[a-z-]+:.*## / {printf "  \033[36m%-12s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

## Setup

setup: setup-rust setup-web ## Install toolchains and dependencies

setup-rust: ## Rust targets, components and wasm-bindgen-cli
	rustup component add rustfmt clippy
	rustup target add wasm32-unknown-unknown aarch64-apple-darwin
	@if [ "$$(wasm-bindgen --version 2>/dev/null | awk '{print $$2}')" != "$(WASM_BINDGEN_VERSION)" ]; then \
		cargo install wasm-bindgen-cli --version $(WASM_BINDGEN_VERSION) --locked; \
	else echo "wasm-bindgen-cli $(WASM_BINDGEN_VERSION) is installed"; fi

setup-web: ## npm packages for the web UI
	cd $(WEB) && npm ci

## Builds

all: cli macos ## The CLI (with the web UI) and the macOS app

cli: web ## target/release/pdfi (embeds apps/web/dist)
	cargo build --release -p pdfi-cli

web: ## The web UI and its wasm build into apps/web/dist
	cd $(WEB) && npm run build

wasm: ## Only the wasm build into apps/web/src/wasm
	scripts/build-wasm.sh

web-dev: ## Vite dev server (pair with `pdfi serve --dev-origin`, see README)
	cd $(WEB) && npm run dev

core: ## Rust core and Swift bindings for the macOS app (release)
	$(MACOS)/scripts/build-core.sh

core-debug: ## Same, debug build of the core
	$(MACOS)/scripts/build-core.sh --debug

macos: core ## The macOS app into apps/macos/build/DerivedData
	xcodebuild -project $(MACOS)/PDFInspector.xcodeproj -scheme PDFInspector \
		-configuration $(CONFIGURATION) -destination 'platform=macOS' -derivedDataPath $(DERIVED) build -quiet
	@echo "$(DERIVED)/Build/Products/$(CONFIGURATION)/PDF Inspector.app"

macos-open: ## Open the Xcode project
	open $(MACOS)/PDFInspector.xcodeproj

fixtures: ## Regenerate tests/fixtures (python3 + fontTools, qpdf)
	tests/fixtures/make.sh

## Checks

test: ## Rust tests
	cargo test --workspace

test-web: web ## Conformance scenario through the wasm build
	cd $(WEB) && npm run test:conformance

test-macos: core-debug ## Swift tests of PdfInspectorKit
	swift test --package-path $(MACOS)/PdfInspectorKit

lint: ## rustfmt check, clippy (native and wasm)
	cargo fmt --all --check
	cargo clippy --workspace --all-targets
	cargo build $(WASM_CRATES) --target wasm32-unknown-unknown
	cargo clippy -p pdfi-wasm --target wasm32-unknown-unknown

fmt: ## Format the Rust code
	cargo fmt --all

deny: ## cargo-deny (cargo install cargo-deny)
	cargo deny check

check: lint test test-web test-macos ## What CI runs (except deny)

clean: ## Remove build outputs
	cargo clean
	rm -rf $(WEB)/dist $(WEB)/src/wasm $(WEB)/*.tsbuildinfo $(MACOS)/build \
		$(MACOS)/PdfInspectorKit/.build $(MACOS)/PdfInspectorKit/Frameworks $(MACOS)/PdfInspectorKit/Sources/PdfiCore
