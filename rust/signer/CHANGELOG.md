# Rust signer changelog

Changes an operator sees when moving from the TS signer (`apps/signer`) to the Rust signer.

## Metrics

Label names are snake_case, the Prometheus convention. The TS signer used camelCase for two
labels, so alert rules and dashboard queries that filter on them must be updated before a Rust
signer takes traffic. A query that should cover both signers during a mixed rollout has to match
both label names.

| Metric              | TS signer label | Rust signer label |
| ------------------- | --------------- | ----------------- |
| `responses`         | `statusCode`    | `status_code`     |
| `full_node_latency` | `codeSegment`   | `code_segment`    |

A query such as `responses{statusCode=~"5.."}` returns nothing for a Rust signer, and a filter
such as `statusCode!="200"` matches every Rust response, because the label is absent.

`full_node_latency` also has a different label value. The TS signer records one sample for each
contract call, labelled `getDataEncryptionKey` or `getOnChainOdisPayments`. The Rust signer
records one sample per account lookup, labelled `getAccount`, which covers both calls.

Other label values keep the TS signer's spelling, such as `operation="insertRequest"`, so one query
covers both signers. `db_ops_instrumentation` has no `incrementQueryCount` or `getAccountExists`
series: the Rust signer records a request and charges its quota in one transaction, measured as
`insertRequest`.

## Database configuration

The database is set with one variable, `DATABASE_URL` (`postgres://user:password@host:5432/database`
or `sqlite:path`). The TS signer's `DB_TYPE`, `DB_HOST`, `DB_PORT`, `DB_USERNAME`, `DB_PASSWORD`,
`DB_DATABASE` and `DB_USE_SSL` are not read.

Three cases stop the signer at startup, so a manifest written for the TS signer cannot start it on an
empty database:

- `DB_HOST` or `DB_TYPE` is set and `DATABASE_URL` is not.
- The key store is not `Mock` and the database is in-memory SQLite, which is the default when
  neither `DATABASE_URL` nor `DB_PATH` is set.
- The Postgres database holds the TS signer's tables (`accountsOnChain` or `requestsOnChain`), they
  were never imported, and `MIGRATE_LEGACY_DATA` is not `true`.

## Database failures

If the lookup for a replayed request fails, `/sign` answers 500 with `CELO_ODIS_ERR_03`. The TS
signer logged the failure and carried on as if the request were new.
