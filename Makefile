PREFIX ?= /usr/local
XDG_DATA_HOME ?= $(HOME)/.local/share
PNL_HOME ?= $(XDG_DATA_HOME)/pnl
CARGO ?= cargo
PHP ?= php
COMPOSER ?= composer
VERSION = $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)

.PHONY: build install test fmt cs analyse validate clean

build:
	$(CARGO) build --release --bins

# Binaries live under $(PNL_HOME)/versions/<version>/bin; $(PNL_HOME)/current
# points at the active version and $(PREFIX)/bin holds symlinks only, so an
# upgrade swaps the `current` link instead of overwriting the binaries.
install: build
	install -d "$(PNL_HOME)/versions/$(VERSION)/bin"
	install -m 0755 target/release/pnl "$(PNL_HOME)/versions/$(VERSION)/bin/pnl"
	install -m 0755 target/release/pnlx "$(PNL_HOME)/versions/$(VERSION)/bin/pnlx"
	ln -sfn "versions/$(VERSION)" "$(PNL_HOME)/current"
	install -d "$(PREFIX)/bin"
	ln -sfn "$(PNL_HOME)/current/bin/pnl" "$(PREFIX)/bin/pnl"
	ln -sfn "$(PNL_HOME)/current/bin/pnlx" "$(PREFIX)/bin/pnlx"

test:
	$(CARGO) test
	$(COMPOSER) test

fmt:
	$(CARGO) fmt

cs:
	$(COMPOSER) cs

analyse:
	$(COMPOSER) analyse

validate:
	$(CARGO) run --bin pnl -- validate

clean:
	$(CARGO) clean
