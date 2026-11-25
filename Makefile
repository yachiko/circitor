.PHONY: verify check-macos

verify:
	cargo fmt --all --check
	cargo clippy --all-targets --features test-endpoint -- -D warnings
	cargo clippy --lib --bins -- -D warnings
	cargo test --features test-endpoint
	$(MAKE) --no-print-directory check-macos

# Checks the macOS build. On a Mac, plain cargo does it. From Linux, aws-lc-sys
# (rustls's crypto provider) needs a C compiler with macOS headers: zig is one,
# driven by cargo-zigbuild. Install both first (README, "Building").
# It checks and doesn't link: linking needs Apple's SDK for the Security and
# CoreFoundation frameworks (rustls-platform-verifier), which zig doesn't ship.
check-macos:
ifeq ($(shell uname -s),Darwin)
	cargo check --target aarch64-apple-darwin --lib --bins
else
	@command -v zig >/dev/null || { echo "check-macos needs zig (README, Building)"; exit 1; }
	@command -v cargo-zigbuild >/dev/null || { echo "check-macos needs cargo-zigbuild (README, Building)"; exit 1; }
	CC_aarch64_apple_darwin="cargo-zigbuild zig cc -- -target aarch64-macos-none" \
		cargo check --target aarch64-apple-darwin --lib --bins
endif
