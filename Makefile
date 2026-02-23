.PHONY: dev check build clean release dashboard-build help

dev: dashboard-build check
	cargo install --path . --locked --force

check:
	cargo fmt --check
	cargo clippy -- -D warnings
	cargo test

build:
	cargo build

dashboard-build:
	cd dashboard && npm install && npm run build

release:
	bash scripts/release.sh $(BUMP)

clean:
	cargo clean

help:
	@echo "make                — dashboard + fmt + clippy + test + install (default)"
	@echo "make check          — fmt + clippy + test"
	@echo "make build          — compile (debug)"
	@echo "make dashboard-build — rebuild frontend dist"
	@echo "make release        — bump version and tag (BUMP=patch|minor|major)"
	@echo "make clean          — remove build artifacts"
