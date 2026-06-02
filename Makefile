# --- Developer targets (require extra tools) ---

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

# --- CI-friendly targets (no extra tools beyond rustup components) ---

ci: lint nextest
	@echo "third-eye-client: CI passed ✓"

lint:
	@echo "third-eye-client: lint\n"
	@cargo fmt --check
	@cargo clippy --all-targets --all-features -- -W clippy::pedantic -D warnings

# --- Test targets ---

nextest:
	@echo "third-eye-client: test (nextest)\n"
	@cargo nextest run

test:
	@echo "third-eye-client: test\n"
	@cargo test

# --- Code coverage ---

nextest-cov:
	@echo "third-eye-client: code coverage (nextest)\n"
	@cargo llvm-cov --open nextest

test-cov:
	@echo "third-eye-client: code coverage\n"
	@cargo llvm-cov --open

coverage:
	@echo "third-eye-client: code coverage (lcov)\n"
	@cargo llvm-cov --lcov --output-path lcov.info nextest
	@echo "Coverage report written to lcov.info"

# --- Misc ---

clean:
	cargo clean

upgrade:
	cargo upgrade --verbose
