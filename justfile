default: check fmt-check clippy test

check:
    cargo check -p odis-signer

test:
    cargo test -p odis-signer

clippy:
    cargo clippy -p odis-signer -- -D warnings

fmt:
    cargo fmt -p odis-signer

fmt-check:
    cargo fmt -p odis-signer -- --check

build:
    cargo build -p odis-signer

build-release:
    cargo build --release -p odis-signer

# Start signer locally with mock keys against Celo Sepolia for E2E testing
run-e2e-signer:
    KEYSTORE_TYPE=Mock \
    PHONE_NUMBER_PRIVACY_API_ENABLED=true \
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
