fmt:
	@echo 'cargo +nightly fmt'
	@script -q -c 'cargo +nightly fmt' /dev/null

dev:
	cargo build

release:
	cargo build --release
