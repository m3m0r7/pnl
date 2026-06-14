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
	$(CARGO) build --release
	$(CARGO) build --release

# Binaries live under $(PNL_HOME)/versions/<version>/bin; $(PNL_HOME)/current
# points at the active version and $(BIN_DIR) holds symlinks only, so an
# upgrade swaps the `current` link instead of overwriting the binaries.
install: build
	install -d "$(PNL_HOME)/versions/$(VERSION)/bin"
	install -m 0755 target/release/pnl "$(PNL_HOME)/versions/$(VERSION)/bin/pnl"
	install -m 0755 target/release/pnlx "$(PNL_HOME)/versions/$(VERSION)/bin/pnlx"
	ln -sfn "versions/$(VERSION)" "$(PNL_HOME)/current"
	install -d "$(BIN_DIR)"
	ln -sfn "$(PNL_HOME)/current/bin/pnl" "$(BIN_DIR)/pnl"
	ln -sfn "$(PNL_HOME)/current/bin/pnlx" "$(BIN_DIR)/pnlx"

test:
	$(CARGO) test
	$(COMPOSER) test

fmt:
	$(CARGO) fmt

cs:
	$(COMPOSER) cs

analyse:
	$(COMPOSER) analyse

# Verify this environment can build pnl: the required toolchain is present.
# Handy as a pre-build gate when building the binaries from source.
validate:
	@command -v "$(CARGO)" >/dev/null 2>&1 || { echo "pnl: cargo not found; install a Rust toolchain (https://rustup.rs) to build pnl" >&2; exit 1; }
	@command -v rustc >/dev/null 2>&1 || { echo "pnl: rustc not found; install a Rust toolchain (https://rustup.rs) to build pnl" >&2; exit 1; }
	@command -v cc >/dev/null 2>&1 || { echo "pnl: a C compiler (cc) is required to build the vendored native dependencies" >&2; exit 1; }
	@echo "toolchain OK: $$($(CARGO) --version) / $$(rustc --version)"

validate-workspace:
	$(CARGO) run --bin pnl -- validate

clean:
	$(CARGO) clean
