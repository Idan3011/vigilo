.PHONY: build dev check clean release help

help:
	@echo "make build    — compile (debug)"
	@echo "make dev      — check + install locally"
	@echo "make check    — fmt + clippy + test"
	@echo "make release  — bump version and tag (BUMP=patch|minor|major)"
	@echo "make clean    — remove build artifacts"

build:
	cargo build

dev: check
	cargo install --path . --locked

check:
	cargo fmt --check
	cargo clippy -- -D warnings
	cargo test

release:
	bash scripts/release.sh $(BUMP)

clean:
	cargo clean
