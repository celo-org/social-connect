# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

SocialConnect is an open-source protocol that maps off-chain personal identifiers (phone numbers, Twitter handles, email, etc.) to on-chain blockchain account addresses on Celo. It uses a federated attestation model with privacy preserved through ODIS (Oblivious Decentralized Identifier Service) for identifier obfuscation.

## Repository Structure

Yarn workspaces monorepo with packages and apps:

**Packages** (`/packages`) - Published to NPM:
- `@celo/phone-number-privacy-common` - Shared library for combiner/signer services
- `@celo/identity` - Main SDK for consuming SocialConnect (identifier obfuscation, attestation lookup/registration)
- `@celo/odis-identifiers` - Identifier prefix definitions and hashing functions
- `@celo/encrypted-backup` - Account recovery SDK powered by ODIS

**Apps** (`/apps`) - Deployed services:
- `signer` - Signs partial threshold BLS signatures for blinded messages
- `combiner` - Orchestrates and combines threshold BLS signatures from signers
- `monitor` - Firebase Cloud Functions for health checks and load testing

## Build Commands

```bash
yarn build              # Build all workspaces
yarn build:fast         # Build only modified packages
yarn clean              # Remove all build artifacts
yarn lint               # ESLint all packages
yarn prettify           # Auto-format code
yarn test               # Run all tests
yarn test:fast          # Test only modified packages
```

## Running Single Package Commands

```bash
yarn workspace @celo/identity run test
yarn workspace @celo/identity run build
yarn --cwd=packages/identity test
yarn --cwd=apps/signer test:coverage
```

## Testing

Jest is configured at root level. Tests are colocated with source in `*.test.ts` or `*.spec.ts` files.

E2E tests run against deployed ODIS instances:
```bash
yarn --cwd=apps/signer test:e2e:celo-sepolia:0-2
yarn --cwd=apps/combiner test:e2e:celo-sepolia
```

## Code Quality

- ESLint flat config (`eslint.config.js`) with TypeScript rules
- Prettier: no semicolons, single quotes, 100 char width
- Pre-commit hooks run prettier and block `DO_NOT_MERGE` markers
- **Important**: Never import `elliptic` directly - use async/dynamic imports only (enforced by ESLint)

## Release Process

Uses changesets for versioning. Every PR impacting consumers needs a changeset:
```bash
yarn cs                 # Create a changeset
```

## Architecture Notes

**App structure** follows domain-driven design:
```
src/
  ├── common/     # Shared utilities, database, key management
  ├── domain/     # Business logic endpoints and services
  └── pnp/        # Phone Number Privacy specific endpoints/services
```

**Database**: Apps use Knex.js with support for PostgreSQL, MySQL, MSSQL, SQLite. Migrations via `db:migrate` scripts.

**Cryptography**: Blind threshold BLS signatures via `blind-threshold-bls` package. HSM support for Azure in signer app.

**Observability**: Bunyan logging, OpenTelemetry tracing, Prometheus metrics.

## Key Dependencies

- `viem` - Modern Ethereum client for blockchain interaction
- `blind-threshold-bls` - BLS threshold signatures (aliased from `@celo/blind-threshold-bls`)
- `io-ts` / `fp-ts` - Runtime type validation and functional programming
- `knex` - Database query builder and migrations

## Rust Signer Port

The Rust signer (`rust/signer`, crate name `odis-signer`) is a near-complete port of the TypeScript signer. PNP endpoints are fully implemented and tested; domain endpoints return 503 stubs.

### Structure

Cargo workspace with root `Cargo.toml` and single member `rust/signer`. The crate is both a binary (`main.rs`) and library (`lib.rs` re-exports all modules).

```
rust/signer/src/
  main.rs                 # Startup: config → router → TCP bind → serve
  server.rs               # Router construction, AppState, startup checks, background pruning task
  config.rs               # Config struct parsed from env vars via dotenvy
  handlers.rs             # HTTP handlers (PNP sign, quota, domain stubs, status)
  types.rs                # Request/response structs, axum extractors
  auth.rs                 # EIP-191 wallet key + DEK (secp256k1/SHA-256) authentication
  crypto.rs               # BLS blind partial signature (bls12-377 G2Scheme)
  errors.rs               # OdisError → HTTP status + JSON error response
  metrics.rs              # Prometheus recorder + http_metrics_layer middleware, upkeep task
  logging.rs              # Log output as text or JSON (LOG_FORMAT)
  account_service/        # Trait + impls: mock, on-chain client, caching (moka), metered
  key_management/         # Trait + impls: mock (dev key shares), Google Secret Manager
  request_service/        # Trait + impls: SQLite (WAL mode), Postgres, legacy TS->canonical ETL, metered
rust/signer/tests/
  pnp_integration.rs      # HTTP tests against the real router with SQLite
  postgres_integration.rs # Postgres backend and ETL against a real Postgres
  data/                   # BLS vectors from the TS library, and the script that writes them
```

### Endpoints

| Route | Status |
|---|---|
| `GET /status` | Implemented |
| `POST /sign` | Implemented (full PNP sign flow) |
| `POST /quotaStatus` | Implemented |
| `POST /domain/sign` | Stub (503) |
| `POST /domain/quotaStatus` | Stub (503) |
| `POST /domain/disable` | Stub (503) |
| `GET /metrics` | Implemented (Prometheus) |

### Build & Test Commands

Use the justfile at the repo root:

```bash
just check          # cargo check
just test           # cargo nextest run
just clippy         # clippy on all targets, tests included, with -D warnings
just fmt            # cargo fmt
just fmt-check      # fmt --check
just build          # debug build
just build-release  # release build (LTO + strip)
just run-e2e-signer # start local signer with mock keys against Celo Sepolia
just test-e2e       # run TS E2E tests against local signer
```

The default `just` target runs: check, fmt-check, clippy, test.

### Docker

`dockerfiles/Dockerfile-signer-rs` — multi-stage build (`rust:1.100.0-trixie` → `debian:trixie-slim`), runs as `nobody`.

Both base images are pinned by digest, like the TS images. The build stage does not read `rust-toolchain.toml`, so the Rust version is set in two places: bump `rust-toolchain.toml` and the `rust:` tag and digest in the Dockerfile together. Keep both stages on the same Debian release, because a binary built on a newer release needs a newer glibc than an older runtime image has. Get a digest with `docker buildx imagetools inspect <image>:<tag>`.

### Parity with the TS signer

A combiner uses Rust and TS signers side by side, so a request the combiner forwards must get the same answer from both: status, error string, response fields, and the same charge to quota. A difference that only someone calling a signer directly can see is acceptable. Every difference an operator or client can see goes into `rust/signer/CHANGELOG.md`, which is also the migration guide for a deployment; update it in the same change.

### Design Decisions
- The signer refuses to start instead of running wrong. With a real key store it needs a persistent database and a full node. It also stops when the full node reports a chain ID other than `CHAIN_ID`, when the key store lacks the latest key, when the Postgres database holds TS tables that were never imported, when `SHOULD_MOCK_ACCOUNT_SERVICE` or `SHOULD_MOCK_REQUEST_SERVICE` is `true`, and when a boolean is not `true` or `false`. Add a new precondition as a startup check, not as a failure on the first request.
- Metric and label names are snake_case. Label values keep the TS signer's spelling where the TS signer has the same series, so one query covers both. Only the API endpoints are counted in the request metrics, not `/status`, `/metrics` or unknown paths. The test `metrics_expose_the_documented_names_and_labels` pins what dashboards and alert rules query; a change there needs a changelog entry.
- SQLite (dev/default) and PostgreSQL (production) are supported; no MySQL/MSSQL. The backend is selected by `DATABASE_URL` (scheme-dispatched: `postgres://` -> Postgres, `sqlite:` -> SQLite), falling back to `DB_PATH` (SQLite, default `:memory:`) when `DATABASE_URL` is unset.
- Both backends share one canonical `requests`/`accounts` schema; addresses are stored EIP-55 checksummed (`Address::to_string()`).
- Postgres can import legacy TS-signer data (`accountsOnChain`/`requestsOnChain`) into the canonical schema when `MIGRATE_LEGACY_DATA=true`: a one-time, marker-guarded, transactional ETL that re-checksums addresses, drops NULL-signature rows, sums quota across address casings, and keeps the latest duplicate request. It never runs twice (guarded by a `legacy_migration` marker row). Without the flag, the signer refuses to start on a database that holds legacy tables but no marker row. See `request_service/legacy_migration.rs`.
- Service traits (`AccountService`, `PnpRequestService`, `KeyProvider`) behind `Arc<dyn Trait>` in `AppState` for testability
- Each service has a metered wrapper adding Prometheus counters/histograms
- Integration tests use `tower::ServiceExt::oneshot` with real SQLite (not mocked); the Postgres backend and ETL are tested against a real Postgres via testcontainers (needs Docker — skipped locally when absent, required in CI via `ODIS_REQUIRE_POSTGRES_TESTS`)

### Conventions
- Workspace layout with `Cargo.toml` at the root, modules in `/rust`
- Prefer common crates: axum, sqlx, serde, tokio, thiserror, anyhow
- Use `thiserror` for library errors, `anyhow` sparingly in main/tests only
- Use `alloy` for Ethereum-related functionality, use the `Address` type for Ethereum addresses and the `Bytes` type for binary data. Also use the `address!` macro to define Ethereum addresses.
- Repository pattern for DB access
- Config via environment variables with `dotenvy` + manual parsing (no complex config frameworks)
- Keep structs flat, derive Debug/Clone/Serialize/Deserialize where useful
- Always add tests, make sure all test cases from the TS signer are covered
- Assert the exact error or the whole response body, not `is_err()` or the status alone. A test that only checks for failure keeps passing when a new check fails first.
- To test a database failure, break a real SQLite file from a second connection (see `database_failures_return_the_ts_error_strings`); do not fake the request service.
- Tests that read a counter compare its value before and after. They are exact under `cargo nextest`, which runs each test in its own process; under plain `cargo test` all tests share one metrics recorder.
- `tests/data/bls_vectors.json` holds what the TS signer's BLS library answers. Do not edit it by hand; regenerate it with `node rust/signer/tests/data/generate_bls_vectors.js`. A change to the `threshold-bls` pin must keep `signatures_equal_the_ts_library` and `edge_inputs_are_answered_as_by_the_ts_library` passing; the one known difference is named in the second test.
- Make sure linting and tests pass, add common tasks to the justfile
