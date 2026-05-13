code:
	@echo "third-eye-client: code check\n"
	@rustup update
	@cargo update
	@cargo upgrade
	@cargo machete
	@cargo audit
	@cargo deny --log-level error check
	@typos
	@cargo fmt
	@cargo fix --allow-dirty --allow-no-vcs --allow-staged
	@cargo clippy --fix --allow-dirty --allow-staged --all-targets --all-features -- -W clippy::pedantic
	@cargo clippy -- -W clippy::pedantic

check: code nextest

clean:
	cargo clean

nextest:
	@echo "third-eye-platform: test\n"
	@cargo nextest run

nextest-cov:
	@echo "third-eye-platform: code coverage\n"
	@cargo llvm-cov --open nextest

test:
	@echo "third-eye-platform: test\n"
	@cargo test

test-cov:
	@echo "third-eye-platform: code coverage\n"
	@cargo llvm-cov --open

upgrade:
	cargo upgrade --verbose
