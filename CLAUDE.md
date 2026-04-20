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
  server.rs               # Router construction, AppState, background pruning task
  config.rs               # Config struct parsed from env vars via dotenvy
  handlers.rs             # HTTP handlers (PNP sign, quota, domain stubs, status)
  types.rs                # Request/response structs, axum extractors
  auth.rs                 # EIP-191 wallet key + DEK (secp256k1/SHA-256) authentication
  crypto.rs               # BLS blind partial signature (bls12-377 G2Scheme)
  errors.rs               # OdisError → HTTP status + JSON error response
  metrics.rs              # Prometheus recorder + http_metrics_layer middleware
  account_service/        # Trait + impls: mock, on-chain client, caching (moka), metered
  key_management/         # Trait + impls: mock (dev key shares), Google Secret Manager
  request_service/        # Trait + impls: SQLite (WAL mode, migrations), metered
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
just test           # cargo test
just clippy         # clippy with -D warnings
just fmt            # cargo fmt
just fmt-check      # fmt --check
just build          # debug build
just build-release  # release build (LTO + strip)
just run-e2e-signer # start local signer with mock keys against Celo Sepolia
just test-e2e       # run TS E2E tests against local signer
```

The default `just` target runs: check, fmt-check, clippy, test.

### Docker

`dockerfiles/Dockerfile-signer-rs` — multi-stage build (`rust:bookworm` → `debian:bookworm-slim`), runs as `nobody`.

### Design Decisions
- SQLite is the only supported database. No PostgreSQL/MySQL support needed.
- Service traits (`AccountService`, `PnpRequestService`, `KeyProvider`) behind `Arc<dyn Trait>` in `AppState` for testability
- Each service has a metered wrapper adding Prometheus counters/histograms
- Integration tests use `tower::ServiceExt::oneshot` with real SQLite (not mocked)

### Conventions
- Workspace layout with `Cargo.toml` at the root, modules in `/rust`
- Prefer common crates: axum, sqlx, serde, tokio, thiserror, anyhow
- Use `thiserror` for library errors, `anyhow` sparingly in main/tests only
- Use `alloy` for Ethereum-related functionality, use the `Address` type for Ethereum addresses and the `Bytes` type for binary data. Also use the `address!` macro to define Ethereum addresses.
- Repository pattern for DB access
- Config via environment variables with `dotenvy` + manual parsing (no complex config frameworks)
- Keep structs flat, derive Debug/Clone/Serialize/Deserialize where useful
- Always add tests, make sure all test cases from the TS signer are covered
- Make sure linting and tests pass, add common tasks to the justfile
