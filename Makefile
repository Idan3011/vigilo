.PHONY: dev check

dev: check
	cargo install --path . --locked

check:
	cargo test
	cargo clippy -- -D warnings
	cargo fmt --check
