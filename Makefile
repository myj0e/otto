CC ?= cc
PKG_CONFIG ?= pkg-config
CARGO ?= cargo

# `make install` is a current-user install by default. Set PREFIX,
# CONFIG_DIR, or STATE_DIR explicitly when a different location is needed.
PREFIX ?=
CONFIG_DIR ?=
STATE_DIR ?=

CPPFLAGS ?=
CPPFLAGS += -Iinclude -D_POSIX_C_SOURCE=200809L
CFLAGS ?= -std=c11 -O2 -Wall -Wextra -Wpedantic

CURL_CFLAGS := $(shell $(PKG_CONFIG) --cflags libcurl 2>/dev/null)
CURL_LIBS := $(shell $(PKG_CONFIG) --libs libcurl 2>/dev/null)
ifeq ($(strip $(CURL_LIBS)),)
CURL_LIBS := -lcurl
endif

# The C implementation is retained as an explicit legacy build. Keeping its
# artifact outside the project root prevents it from being confused with the
# formal Rust `otto` binary.
C_TARGET := build/c/otto-c
C_SOURCES := $(wildcard src/*.c)
C_HEADERS := $(wildcard include/otto/*.h)

RUST_MANIFEST := rust/otto/Cargo.toml
RUST_RELEASE_BINARY := target/release/otto

INSTALL_ARGS = $(if $(strip $(PREFIX)),--prefix "$(PREFIX)") \
               $(if $(strip $(CONFIG_DIR)),--config-dir "$(CONFIG_DIR)") \
               $(if $(strip $(STATE_DIR)),--state-dir "$(STATE_DIR)")
UNINSTALL_ARGS = $(if $(strip $(STATE_DIR)),--state-dir "$(STATE_DIR)")

.PHONY: all check-deps check-rust c-build c-test rust-build rust-test rust-fmt \
        rust-functional-test test clean install uninstall

# Formal build entry: Rust is now the maintained and installed implementation.
all: rust-build

check-deps:
	@command -v $(CC) >/dev/null 2>&1 || \
		(echo "error: C compiler not found" >&2; exit 1)
	@command -v $(PKG_CONFIG) >/dev/null 2>&1 || \
		(echo "error: pkg-config not found" >&2; exit 1)
	@$(PKG_CONFIG) --exists libcurl || \
		(echo "error: libcurl development package not found" >&2; exit 1)

check-rust:
	@command -v $(CARGO) >/dev/null 2>&1 || \
		(echo "error: cargo not found; install the Rust toolchain first" >&2; exit 1)

c-build: check-deps
	@mkdir -p "$(dir $(C_TARGET))"
	$(CC) $(CPPFLAGS) $(CFLAGS) $(CURL_CFLAGS) -o $(C_TARGET) $(C_SOURCES) $(CURL_LIBS)

c-test: c-build
	sh ./tests/test.sh ./$(C_TARGET)

rust-build: check-rust
	$(CARGO) build --release --manifest-path $(RUST_MANIFEST) --locked

rust-test: check-rust
	$(CARGO) test --manifest-path $(RUST_MANIFEST) --locked

rust-fmt: check-rust
	$(CARGO) fmt --manifest-path $(RUST_MANIFEST) --all -- --check

rust-functional-test: rust-build
	sh ./tests/test.sh ./$(RUST_RELEASE_BINARY)
	python3 ./tests/test_agent.py ./$(RUST_RELEASE_BINARY)
	sh ./tests/test_install.sh ./$(RUST_RELEASE_BINARY)

test: rust-fmt rust-test rust-functional-test c-test

clean:
	rm -f -- "$(C_TARGET)"
	@if command -v $(CARGO) >/dev/null 2>&1; then \
		$(CARGO) clean --manifest-path $(RUST_MANIFEST); \
	fi

# The scripts remain the compatibility implementation for users who already
# call them directly. Make targets are the documented lifecycle interface.
install: rust-build
	OTTO_INSTALL_BINARY="$(CURDIR)/$(RUST_RELEASE_BINARY)" \
		./scripts/install.sh $(INSTALL_ARGS)

uninstall:
	./scripts/uninstall.sh $(UNINSTALL_ARGS)
