PREFIX ?= /usr/local
BIN_DIR ?= $(PREFIX)/bin
XDG_DATA_HOME ?= $(HOME)/.local/share
PNL_HOME ?= $(XDG_DATA_HOME)/pnl
CARGO ?= cargo
PHP ?= php
COMPOSER ?= composer
VERSION = $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)

.PHONY: build install test fmt cs analyse validate validate-workspace clean

# Two passes: the first produces the support cdylib, the second embeds it into
# the pnl/pnlx binaries (see build.rs). Built without --bins so the cdylib is
# produced alongside the binaries.
build:
	@echo "==> build: release pass 1/2 — produce the support cdylib (build.rs embeds a placeholder here)"
	$(CARGO) build --release
	@echo "==> build: release pass 2/2 — rebuild so build.rs embeds the cdylib into pnl/pnlx"
	$(CARGO) build --release
	@echo "==> build: done -> target/release/pnl, target/release/pnlx"

# Binaries live under $(PNL_HOME)/versions/<version>/bin; $(PNL_HOME)/current
# points at the active version and $(BIN_DIR) holds symlinks only, so an
# upgrade swaps the `current` link instead of overwriting the binaries.
install: build
	@echo "==> install: copying pnl/pnlx $(VERSION) into $(PNL_HOME)/versions/$(VERSION)/bin"
	install -d "$(PNL_HOME)/versions/$(VERSION)/bin"
	install -m 0755 target/release/pnl "$(PNL_HOME)/versions/$(VERSION)/bin/pnl"
	install -m 0755 target/release/pnlx "$(PNL_HOME)/versions/$(VERSION)/bin/pnlx"
	@echo "==> install: pointing $(PNL_HOME)/current at versions/$(VERSION)"
	ln -sfn "versions/$(VERSION)" "$(PNL_HOME)/current"
	@echo "==> install: linking $(BIN_DIR)/pnl and $(BIN_DIR)/pnlx -> current"
	install -d "$(BIN_DIR)"
	ln -sfn "$(PNL_HOME)/current/bin/pnl" "$(BIN_DIR)/pnl"
	ln -sfn "$(PNL_HOME)/current/bin/pnlx" "$(BIN_DIR)/pnlx"
	@echo "==> install: done — make sure $(BIN_DIR) is on your PATH"

test:
	@echo "==> test: Rust unit/integration tests (cargo test)"
	$(CARGO) test
	@echo "==> test: PHP tests (phpunit, via composer)"
	$(COMPOSER) test

fmt:
	@echo "==> fmt: formatting Rust sources (cargo fmt)"
	$(CARGO) fmt

cs:
	@echo "==> cs: checking PHP code style (php-cs-fixer, via composer)"
	$(COMPOSER) cs

analyse:
	@echo "==> analyse: PHP static analysis (phpstan level max, via composer)"
	$(COMPOSER) analyse

# Verify this environment can build pnl: the required toolchain is present.
# Handy as a pre-build gate when building the binaries from source.
validate:
	@echo "==> validate: checking the build toolchain (cargo, rustc, cc)"
	@command -v "$(CARGO)" >/dev/null 2>&1 || { echo "pnl: cargo not found; install a Rust toolchain (https://rustup.rs) to build pnl" >&2; exit 1; }
	@command -v rustc >/dev/null 2>&1 || { echo "pnl: rustc not found; install a Rust toolchain (https://rustup.rs) to build pnl" >&2; exit 1; }
	@command -v cc >/dev/null 2>&1 || { echo "pnl: a C compiler (cc) is required to build the vendored native dependencies" >&2; exit 1; }
	@echo "==> validate: toolchain OK — $$($(CARGO) --version) / $$(rustc --version)"

validate-workspace:
	@echo "==> validate-workspace: validating pnl.json, the lockfile, and the pathmap against their schemas"
	$(CARGO) run --bin pnl -- validate

clean:
	@echo "==> clean: removing the Cargo target/ directory (cargo clean)"
	$(CARGO) clean
