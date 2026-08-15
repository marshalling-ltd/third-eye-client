# --- Developer targets (require extra tools) ---

code:
	@echo "third-eye-client: code check\n"
	@rustup update
	@rustup run nightly cargo update
	@rustup run nightly cargo upgrade
	@rustup run nightly cargo machete
	@rustup run nightly cargo audit
	@rustup run nightly cargo deny --log-level error check
	@typos
	@rustup run nightly cargo fmt
	@rustup run nightly cargo fix --allow-dirty --allow-no-vcs --allow-staged
	@rustup run nightly cargo clippy --fix --allow-dirty --allow-staged --all-targets --all-features -- -W clippy::pedantic
	@rustup run nightly cargo clippy -- -W clippy::pedantic
	@rustup run nightly cargo test --doc

check: code nextest

# --- Test targets ---

nextest:
	@echo "third-eye-client: test (nextest)\n"
	@rustup run nightly cargo nextest run --test-threads=1

test:
	@echo "third-eye-client: test\n"
	@rustup run nightly cargo test

# --- Code coverage ---

nextest-cov:
	@echo "third-eye-client: code coverage (nextest)\n"
	@rustup run nightly cargo llvm-cov --open nextest --test-threads=1

test-cov:
	@echo "third-eye-client: code coverage\n"
	@rustup run nightly cargo llvm-cov --open

coverage:
	@echo "third-eye-client: code coverage (lcov)\n"
	@rustup run nightly cargo llvm-cov --lcov --output-path lcov.info nextest --test-threads=1
	@echo "Coverage report written to lcov.info"
	
# --- Misc ---

clean:
	cargo clean

upgrade:
	@cargo upgrade --verbose

bump-patch:
	@bash ./scripts/bump_patch_version.sh

open-api:
	@openapi-generator-cli generate -i https://third-eye.marshalling.eu/api/v1/api-doc/openapi.json -g rust -o ./generated --skip-validate-spec

requirements:
	@echo "third-eye-client: requirements\n"
	@rustup update
	@rustup install nightly
	@cargo install cargo-audit
	@cargo install cargo-deny
	@cargo install cargo-edit
	@cargo install cargo-llvm-cov
	@cargo install cargo-machete
	@cargo install cargo-nextest --locked
	@cargo install typos-cli
