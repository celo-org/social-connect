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
yarn --cwd=apps/signer test:e2e:alfajores:0-2
yarn --cwd=apps/combiner test:e2e:alfajores
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

## Rust port

There's a WIP Rust port of the signer. The Rust port aims to provide a more efficient and secure implementation of the application's core functionalities. The Rust port is currently in development and will be available soon.

### Design decisions
- SQLite is the only supported database. No PostgreSQL/MySQL support needed.

### Conventions
- Use a workspace layout with `Cargo.toml` at the root modules in the `/rust` directory.
- Prefer common crates: axum, sqlx, serde, tokio, thiserror, anyhow
- Use `thiserror` for library errors, `anyhow` sparingly in main/tests only
- Use `alloy` for Ethereum-related functionality, use the `Address` type for Ethereum addresses and the `Bytes` type for binary data. Also use the `address!` macro to define Ethereum addresses.
- Use a repository pattern for DB access
- Config via environment variables with `dotenvy` + manual parsing (no complex config frameworks)
- Keep structs flat, derive Debug/Clone/Serialize/Deserialize where useful
- Always add tests, make sure all test cases from the TS signer are copied.
- Make sure linting and tests pass, add common tasks to the justfile
