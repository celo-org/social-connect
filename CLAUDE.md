# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

SocialConnect is an open-source protocol that maps off-chain personal identifiers (phone numbers, Twitter handles, email, etc.) to on-chain blockchain account addresses on Celo. It uses a federated attestation model with privacy preserved through ODIS (Oblivious Decentralized Identifier Service) for identifier obfuscation.

## Repository Structure

Yarn workspaces monorepo with packages and apps:

**Packages** (`/packages`) - Published to NPM:
- `@celo/identity` - SDK for SocialConnect: identifier obfuscation through ODIS, attestation
  lookup and registration, the identifier prefixes and hashing, and the ODIS PNP request and
  response types

**Apps** (`/apps`):
- `monitor` - Firebase Cloud Functions for health checks and load testing

**Services** (`/rust`): the ODIS signer and combiner, see the Rust sections below.

**End-to-end suites** (`/e2e`, private workspace `odis-e2e`): PNP tests against a running signer
or combiner, whichever implementation.

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
```

## Testing

Jest is configured at root level. Tests are colocated with source in `*.test.ts` or `*.spec.ts` files.

E2E tests run against deployed ODIS instances. Set the version variable to the version the
service under test reports at `/status`; the suite compares it with the version the service
reports in its answers:
```bash
DEPLOYED_SIGNER_SERVICE_VERSION=<version> yarn --cwd=e2e test:signer:celo-sepolia:0   # also :1 and :2
DEPLOYED_COMBINER_SERVICE_VERSION=<version> yarn --cwd=e2e test:combiner:celo-sepolia
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

The TS signer and combiner are released from `release/odis-ts-4.x`; see RELEASE.md.

## Commit Messages

Subjects follow [Scoped Commits](https://scopedcommits.com): `<scope>: <description>`, for example `monitor: set CHAIN_ID for the mainnet deploy`. There is no type prefix such as `feat` or `fix`.

- The scope names the component the commit changes. An app, package or workspace takes its directory name (`monitor`, `identity`, `e2e`). A Rust crate takes its directory name under `rust/` plus `-rs` (`signer-rs`, `combiner-rs`), which keeps it apart from the TS app of the same name on `release/odis-ts-4.x`. Changes outside the components use `ci`, `deps` or `docs`.
- A commit that changes two components names both, separated by a comma. One that changes the whole repo uses `treewide`.
- Merges and reverts keep Git's default subject.
- The description is imperative, starts in lower case and has no full stop. It does not repeat the scope: `signer-rs: add JSON log output`, not `signer-rs: add JSON log output to the Rust signer`.

## Architecture Notes

**Cryptography**: Blind threshold BLS signatures via `blind-threshold-bls`.

## Key Dependencies

- `viem` - Modern Ethereum client for blockchain interaction
- `blind-threshold-bls` - BLS threshold signatures (aliased from `@celo/blind-threshold-bls`)
- `io-ts` / `fp-ts` - Runtime type validation and functional programming

## TS signer and combiner

- The TS signer and combiner live on `release/odis-ts-4.x`, not on `main`. Fixes go to that
  branch and are released from there with a tag (RELEASE.md). Never restore their code to `main`.
- Refer to TS code by tag, which never moves: `git show odis-signer-4.0.6:apps/signer/...`,
  `git show odis-combiner-4.0.5:apps/combiner/...`.
- A fix that also applies to the Rust services gets ported to `main`.
- Requests the SDK builds must be accepted unchanged by the TS signers that operators still run
  and by the Rust services.

## Libraries

- `@celo/encrypted-backup`, `@celo/phone-number-privacy-common` and the ODIS domain API are
  sunset. Don't bring them back, and don't add domain features.
- Add a `resolutions` pin only for a security fix that the direct dependency hasn't picked up
  yet, and remove it once it has. A pin freezes the version, so a later advisory against it
  stays unfixed.

## Rust Signer Port

The Rust signer (`rust/signer`, crate name `odis-signer`) replaces the TypeScript signer (last release: tag `odis-signer-4.0.6`). PNP endpoints are fully implemented and tested; domain endpoints return 503 stubs.

### Structure

Cargo workspace with root `Cargo.toml` and two members, `rust/signer` and `rust/combiner`. The crate is both a binary (`main.rs`) and library (`lib.rs` re-exports all modules).

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
  startup.rs              # The built binary's refusals to start, as the operator reads them
  proxy.rs                # Full node lookups go through the proxy, as one test in its own process
  data/                   # BLS and wallet-key vectors from the TS libraries, and the scripts that write them
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
just test-e2e       # run the PNP suite in `e2e/` against the local signer
```

The default `just` target runs: check, fmt-check, clippy, test.

### Docker

`dockerfiles/Dockerfile-signer-rs` — multi-stage build (`rust:1.100.0-trixie` → `debian:trixie-slim`), runs as `nobody`.

Both base images are pinned by digest. The build stage does not read `rust-toolchain.toml`, so the Rust version is set in two places: bump `rust-toolchain.toml` and the `rust:` tag and digest in the Dockerfile together. Keep both stages on the same Debian release, because a binary built on a newer release needs a newer glibc than an older runtime image has. Get a digest with `docker buildx imagetools inspect <image>:<tag>`.

### Parity with the TS signer

A combiner uses Rust and TS signers side by side, so a request the combiner forwards must get the same answer from both: status, error string, response fields, and the same charge to quota. A difference that only someone calling a signer directly can see is acceptable. Every difference an operator or client can see goes into `rust/signer/CHANGELOG.md`, which is also the migration guide for a deployment; update it in the same change.

### Design Decisions
- The signer refuses to start instead of running wrong. With a real key store it needs a persistent database and a full node. It also stops when the full node reports a chain ID other than `CHAIN_ID`, when the key store lacks the latest key, when the Postgres database holds TS tables that were never imported, when `SHOULD_MOCK_ACCOUNT_SERVICE` or `SHOULD_MOCK_REQUEST_SERVICE` is `true`, and when a boolean is not `true` or `false`. Add a new precondition as a startup check, not as a failure on the first request.
- Metric and label names are snake_case. Label values keep the TS signer's spelling where the TS signer has the same series, so one query covers both. Only the API endpoints are counted in the request metrics, not `/status`, `/metrics` or unknown paths. The test `metrics_expose_the_documented_names_and_labels` pins what dashboards and alert rules query; a change there needs a changelog entry.
- SQLite (dev/default) and PostgreSQL (production) are supported; no MySQL/MSSQL. The backend is selected by `DATABASE_URL` (scheme-dispatched: `postgres://` -> Postgres, `sqlite:` -> SQLite), falling back to `DB_PATH` (SQLite, default `:memory:`) when `DATABASE_URL` is unset.
- Both backends share one canonical `requests`/`accounts` schema; addresses are stored EIP-55 checksummed (`Address::to_string()`).
- A query is charged once per account, so `requests` has one row per `(caller_address, blinded_query)`, never one per key version. The row keeps the signature and the key version that made it (`key_version`, NULL when unknown: TS imports). `/sign` serves the stored signature only when that version equals the request's resolved version; otherwise it signs again and `record_request` reports the query as already charged. Do not infer a version for a NULL row.
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
- Always add tests, make sure all test cases from the TS signer (tag `odis-signer-4.0.6`) are covered
- Assert the exact error or the whole response body, not `is_err()` or the status alone. A test that only checks for failure keeps passing when a new check fails first.
- To test a database failure, break a real SQLite file from a second connection (see `database_failures_return_the_ts_error_strings`); do not fake the request service.
- Tests that read a counter compare its value before and after. They are exact under `cargo nextest`, which runs each test in its own process; under plain `cargo test` all tests share one metrics recorder.
- `tests/data/bls_vectors.json` holds what the TS signer's BLS library answers. Do not edit it by hand; regenerate it with `node rust/signer/tests/data/generate_bls_vectors.js`. A change to the `threshold-bls` pin must keep `signatures_equal_the_ts_library` and `edge_inputs_are_answered_as_by_the_ts_library` passing; the one known difference is named in the second test.
- `tests/data/wallet_key_vectors.json` holds wallet-key signatures made by `@celo/utils` and by viem, and what the TS services answer for each. Do not edit it by hand; regenerate it with `node rust/signer/tests/data/generate_wallet_key_vectors.js`. The wallet-key hash counts the body length in UTF-16 code units, as `@celo/utils` does, not in bytes as EIP-191 says.
- Make sure linting and tests pass, add common tasks to the justfile

## Rust Combiner Port

The Rust combiner (`rust/combiner`, crate name `odis-combiner`) is a port of the TypeScript combiner's PNP endpoints (last release: tag `odis-combiner-4.0.5`). It depends on `odis-signer` for authentication, logging, env helpers and request types (`AuthenticationMethod`, `KEY_VERSION_HEADER`, `is_valid_blinded_phone_number`). The domain endpoints return 503 stubs. The crate is both a binary (`main.rs`) and a library (`lib.rs` re-exports all modules).

### Structure

```
rust/combiner/src/
  main.rs                 # Startup: config → recorder → router → TCP bind → serve
  server.rs               # Router construction, CORS, body limit, startup check, panic handler
  config.rs               # Config from env vars; every setting that would run wrong stops the start
  handlers.rs             # HTTP handlers (status, domain stubs, /sign, /quotaStatus)
  body.rs                 # Request body parsed and written back as JSON.stringify writes it
  request.rs              # Request field checks and key version resolution, as TS does them
  auth.rs                 # Wallet key + DEK authentication over the canonical body
  dek.rs                  # DEK lookup from the Accounts contract, with a short cache
  signers.rs              # Concurrent signer calls under one deadline, answer classification
  signature.rs            # BLS combine and verify, key versions, share index binding
  quota.rs                # Quota state and discrepancy warnings over the signers' answers
  errors.rs               # TS error and warning strings, byte for byte
  metrics.rs              # Metric names, Prometheus recorder, request metrics layer
rust/combiner/tests/
  sign.rs                 # /sign in front of real Rust signers
  quota.rs                # /quotaStatus in front of real Rust signers
  server.rs               # Routes, CORS, transport answers, startup check
  metrics.rs              # Metric names, labels and buckets against the TS combiner's
  recorder.rs             # The global recorder, in a process of its own
  startup.rs              # The built binary's refusals to start, as the operator reads them
  proxy.rs                # Signer calls and full node lookups go through the proxy, as one test
  differential/           # the released TS combiner image and this combiner on the same requests
  common/                 # Shared test helpers
  data/                   # BLS combine and canonical body vectors, and the scripts that write them
```

### Endpoints

| Route | Status |
|---|---|
| `GET /status` | Implemented |
| `POST /sign` | Implemented |
| `POST /quotaStatus` | Implemented |
| `POST /domain/sign` | Stub (503) |
| `POST /domain/quotaStatus` | Stub (503) |
| `POST /domain/disable` | Stub (503) |
| `GET /metrics` | Implemented (Prometheus) |

### Build & Test Commands

The signer's recipes cover the combiner too (`just`, `just test`, `just clippy`, `just fmt`, since they run on the workspace). In addition:

```bash
just check-tls                                   # fail if Cargo.lock holds a TLS stack other than rustls
just test-differential                           # compare with the released TS combiner image on the same requests; needs Docker
just test-e2e-combiner <url> <version>           # TS end-to-end suite against the combiner at <url> that reports <version>
```

`test-e2e-combiner` runs `yarn --cwd=e2e test:combiner:celo-sepolia` on Celo Sepolia, with `ODIS_COMBINER_SERVICE_URL` and `DEPLOYED_COMBINER_SERVICE_VERSION` set from the arguments. It spends quota of a shared test account.

### Docker

`dockerfiles/Dockerfile-combiner-rs` is built like the signer's image: multi-stage (`rust:1.100.0-trixie` → `debian:trixie-slim`), both base images pinned by digest, runs as `nobody`. The signer's rules for bumping the Rust version apply to it as well.

### Parity with the TS combiner

A client and an operator must see the TS combiner's answers: status, body, error strings, metric names. Every difference they can see goes into `rust/combiner/CHANGELOG.md`, which is also the migration guide for a deployment; update it in the same change.

### Design Decisions
- The signed body is the canonical one: the combiner parses the client's JSON and writes it back as `JSON.stringify` does, and the signature check, the length limit and the signer calls all use that text. A body the writer cannot reproduce exactly as JavaScript would (non-integer numbers, duplicate or digit-only keys, lone surrogates, a BOM, deep nesting) is refused with 400 `WARN_01`. Otherwise signers would check the signature over a different string than the combiner did, and some would charge a request that then fails.
- A signer entry may carry a `shareIndex`. A partial signature whose index differs from it is dropped before it counts, and the log and `combiner_bls_compute_errors_total` name that signer. The field is optional, and one index applies to every key version. A wrong index makes signers charge for requests whose partials the combiner drops, and no startup check can catch it, so the changelog says how to read an index from a signer's own answers.
- The combiner refuses to start instead of running wrong, as the signer does. Add a new precondition as a startup check, not as a failure on the first request.
- The checks the TS combiner makes and the Rust signer does not carry the marker `TODO(all-rust-signers):` with a line on why, so they can all be found and removed once every signer makes them. `rg 'TODO\(all-rust-signers'`.
- Metric names are the TS combiner's, label names snake_case. The recorder is global to the process. Outside tests, only `main.rs` installs it, and the router takes the handle. A test installs the combiner's recorder only in a process with no in-process Rust signer: its own test binary (`tests/recorder.rs`, `tests/metrics.rs`) or once per process through a `OnceLock` (`tests/server.rs`). Tests that run Rust signers in-process use `odis_signer::metrics::install_recorder()` (`tests/common/mod.rs`).
- Tests run the combiner in front of real `odis-signer` routers on real ports; misbehaving signers are small axum servers. Assert the exact error string or the whole body.
- `tests/data/canonical_body_vectors.json` and `tests/data/combine_vectors.json` hold what the TS code answers. Do not edit them by hand; regenerate with `node rust/combiner/tests/data/generate_canonical_body_vectors.js` and `node rust/combiner/tests/data/generate_combine_vectors.js`.
- The differential test runs the TS combiner from its released image, pinned by digest, in front of in-process Rust signers. It skips without Docker, and CI requires it.
