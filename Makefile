.PHONY: verify check-macos

verify:
	cargo fmt --all --check
	cargo clippy --all-targets --features test-endpoint -- -D warnings
	cargo clippy --lib --bins -- -D warnings
	cargo test --features test-endpoint

# Not part of verify: from Linux, aws-lc-sys (rustls's crypto provider) stops
# with "cc: error: unrecognized command-line option '-mmacosx-version-min=11.0'".
# Run it by hand on a machine with a macOS C toolchain; add the darwin target
# to rust-toolchain.toml first.
check-macos:
	cargo check --target aarch64-apple-darwin --lib --bins
