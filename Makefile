CARGO ?= cargo

# `make install` is a current-user install by default. Set PREFIX,
# CONFIG_DIR, or STATE_DIR explicitly when a different location is needed.
PREFIX ?=
CONFIG_DIR ?=
STATE_DIR ?=

RUST_MANIFEST := Cargo.toml
RUST_RELEASE_BINARY := target/release/otto

INSTALL_ARGS = $(if $(strip $(PREFIX)),--prefix "$(PREFIX)") \
               $(if $(strip $(CONFIG_DIR)),--config-dir "$(CONFIG_DIR)") \
               $(if $(strip $(STATE_DIR)),--state-dir "$(STATE_DIR)")
UNINSTALL_ARGS = $(if $(strip $(STATE_DIR)),--state-dir "$(STATE_DIR)")

.PHONY: all check-rust rust-build rust-test rust-fmt \
	rust-functional-test test clean install uninstall

# Rust is the maintained and installed implementation.
all: rust-build

check-rust:
	@command -v $(CARGO) >/dev/null 2>&1 || \
		(echo "error: cargo not found; install the Rust toolchain first" >&2; exit 1)

rust-build: check-rust
	$(CARGO) build --release --manifest-path $(RUST_MANIFEST) --locked

rust-test: check-rust
	$(CARGO) test --manifest-path $(RUST_MANIFEST) --locked

rust-fmt: check-rust
	$(CARGO) fmt --manifest-path $(RUST_MANIFEST) --all -- --check

rust-functional-test: rust-build
	sh ./tests/test.sh ./$(RUST_RELEASE_BINARY)
	python3 ./tests/test_agent.py ./$(RUST_RELEASE_BINARY)
	python3 ./tests/test_websearch.py ./$(RUST_RELEASE_BINARY)
	sh ./tests/test_install.sh ./$(RUST_RELEASE_BINARY)

test: rust-fmt rust-test rust-functional-test

clean:
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
