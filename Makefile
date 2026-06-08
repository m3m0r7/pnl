PREFIX ?= /usr/local
CARGO ?= cargo
PHP ?= php
COMPOSER ?= composer

.PHONY: build install test fmt cs analyse validate clean

build:
	$(CARGO) build --release --bins

install: build
	install -d "$(PREFIX)/bin"
	install -m 0755 target/release/pnl "$(PREFIX)/bin/pnl"
	install -m 0755 target/release/pnlx "$(PREFIX)/bin/pnlx"

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
