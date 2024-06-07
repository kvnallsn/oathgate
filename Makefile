
debug:
	cargo build --package oathgate-bridge
	cargo build --package oathgate-runner
	cargo build --package oathgate-fabrial --target=x86_64-unknown-linux-musl

release:
	cargo build --release --package oathgate-bridge
	cargo build --release --package oathgate-runner
	cargo build --release --package oathgate-fabrial --target=x86_64-unknown-linux-musl
