# Local equivalents of the GitHub Actions verify job.
# `just deny` needs cargo-deny on PATH (CI installs 0.20.2).

fmt:
	cargo fmt --all

lint:
	cargo clippy --all-targets --all-features -- -D warnings

test:
	cargo test --all-targets --all-features

deny:
	cargo deny check

ci:
	cargo fmt --all -- --check
	cargo clippy --all-targets --all-features -- -D warnings
	cargo test --all-targets --all-features
