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
