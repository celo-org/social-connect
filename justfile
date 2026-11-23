default: check fmt-check clippy check-tls test

check:
    cargo check --workspace

test:
    cargo nextest run --workspace

# Run the Postgres/ETL integration tests, requiring Docker (fails if unavailable)
test-postgres:
    ODIS_REQUIRE_POSTGRES_TESTS=1 cargo nextest run -p odis-signer --test postgres_integration

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

# Fails if Cargo.lock holds a TLS stack other than rustls.
check-tls:
    awk '/^name = "(native-tls|openssl|hyper-tls)"$/ { print "forbidden TLS crate in Cargo.lock: " $3; found = 1 } END { exit found }' Cargo.lock

build:
    cargo build --workspace

build-release:
    cargo build --release --workspace

# Compare the Rust combiner with the TS combiner on the same requests. Needs `yarn build` first.
test-differential:
    cargo build -p odis-combiner
    ODIS_RUST_COMBINER_BIN="$PWD/target/debug/odis-combiner" corepack yarn --cwd=apps/combiner test:differential

# Run the PNP end-to-end tests against the combiner at `url`, which reports `version`.
test-e2e-combiner url version:
    CONTEXT_NAME=celo-sepolia \
    ODIS_BLOCKCHAIN_PROVIDER=https://forno.celo-sepolia.celo-testnet.org \
    ODIS_COMBINER_SERVICE_URL={{url}} \
    DEPLOYED_COMBINER_SERVICE_VERSION={{version}} \
    corepack yarn --cwd=apps/combiner test:e2e:pnp

# Start signer locally with mock keys against Celo Sepolia for E2E testing
run-e2e-signer:
    KEYSTORE_TYPE=Mock \
    BLOCKCHAIN_PROVIDER=https://forno.celo-sepolia.celo-testnet.org \
    CHAIN_ID=11142220 \
    DB_PATH=:memory: \
    RUST_LOG=info \
    cargo run -p odis-signer

# Run TS E2E tests against a local signer (start with `just run-e2e-signer` first)
test-e2e:
    cd apps/signer && \
    CONTEXT_NAME=local \
    ODIS_SIGNER_SERVICE_URL=http://localhost:8080 \
    DEPLOYED_SIGNER_SERVICE_VERSION=0.1.0 \
    LOCAL_PHONE_NUMBER_PRIVACY_POLYNOMIAL=0200000000000000eb569eb8701d831a0a9ffe5df325eed2ac7f5693c758c02aad5804009fcca42ece73f7f560b3e237b60a6b0f5e0d6501516b38b0f9fa20dd944c66edf798cfea73bc232983e0885b04e3632dc125b2ad63c13676723b931d5f686d2c83165d817aaff1f84d0b008ad218eff19db698f343168cf931ba8347640123a2f826f62b66ff084273f494d4647758e9a9f889009d573705824a0e74e1f49ed234462058e53bbb4fef370b55f78da89df070c661782a84239b8c7623d09e34b9f91f7781 \
    npx jest test/end-to-end/pnp.test.ts --forceExit
