default: check fmt-check clippy test

check:
    cargo check -p odis-signer

test:
    cargo nextest run -p odis-signer

clippy:
    cargo clippy -p odis-signer --all-targets -- -D warnings

fmt:
    cargo fmt -p odis-signer

fmt-check:
    cargo fmt -p odis-signer -- --check
