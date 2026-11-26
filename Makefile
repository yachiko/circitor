.PHONY: verify dist

VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
# The Rust target to package; the release workflow sets it per platform.
TARGET  ?= $(shell rustc -vV | sed -n 's/^host: //p')
OS      := $(if $(findstring apple-darwin,$(TARGET)),darwin,linux)
ARCH    := $(if $(findstring aarch64,$(TARGET)),arm64,amd64)
# The tap's naming, as the owner's other tools use it: name_version_os_arch.
NAME    := circitor_$(VERSION)_$(OS)_$(ARCH)
DIST    := dist

verify:
	cargo fmt --all --check
	cargo clippy --all-targets --features test-endpoint -- -D warnings
	cargo clippy --lib --bins -- -D warnings
	cargo test --features test-endpoint

# The release archive for TARGET: dist/circitor_VERSION_OS_ARCH.tar.gz, and the
# bare binary at dist/circitor for a smoke test. Built without test-endpoint
# (TST-1); the grep fails the build if test code got in anyway.
dist:
	cargo build --release --locked --target $(TARGET)
	@! grep -qa CIRCITOR_TEST_ target/$(TARGET)/release/circitor || { echo "the release binary holds test code (TST-1)"; exit 1; }
	rm -rf $(DIST)/$(NAME)
	mkdir -p $(DIST)/$(NAME)
	cp target/$(TARGET)/release/circitor LICENSE README.md $(DIST)/$(NAME)/
	cp target/$(TARGET)/release/circitor $(DIST)/circitor
	tar -czf $(DIST)/$(NAME).tar.gz -C $(DIST)/$(NAME) circitor LICENSE README.md
	rm -rf $(DIST)/$(NAME)
