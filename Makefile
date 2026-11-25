.PHONY: verify check-macos

verify:
	cargo fmt --all --check
	cargo clippy --all-targets --features test-endpoint -- -D warnings
	cargo clippy --lib --bins -- -D warnings
	cargo test --features test-endpoint
	$(MAKE) --no-print-directory check-macos

# Builds for macOS. On a Mac, plain cargo does it. From Linux, aws-lc-sys
# (rustls's crypto provider) needs a C compiler with macOS headers: zig is one,
# driven by cargo-zigbuild. Install both first (README, "Building").
check-macos:
ifeq ($(shell uname -s),Darwin)
	cargo check --target aarch64-apple-darwin --lib --bins
else
	@command -v zig >/dev/null || { echo "check-macos needs zig (README, Building)"; exit 1; }
	@command -v cargo-zigbuild >/dev/null || { echo "check-macos needs cargo-zigbuild (README, Building)"; exit 1; }
	cargo zigbuild --target aarch64-apple-darwin --lib --bins
endif
