.PHONY: dev check build clean release help

dev: check
	cargo install --path . --locked --force

check:
	cargo fmt --check
	cargo clippy -- -D warnings
	cargo test

build:
	cargo build

release:
	bash scripts/release.sh $(BUMP)

clean:
	cargo clean

help:
	@echo "make          — fmt + clippy + test + install (default)"
	@echo "make check    — fmt + clippy + test"
	@echo "make build    — compile (debug)"
	@echo "make release  — bump version and tag (BUMP=patch|minor|major)"
	@echo "make clean    — remove build artifacts"
