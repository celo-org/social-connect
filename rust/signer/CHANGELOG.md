# Rust signer changelog

Changes an operator sees when moving from the TS signer (`apps/signer`) to the Rust signer.

## Migrating a deployment

A manifest written for the TS signer needs these changes before it can run the Rust signer. The
sections below give the details.

| Variable              | Change                                                                       |
| --------------------- | ---------------------------------------------------------------------------- |
| `DATABASE_URL`        | Add. It replaces the `DB_*` variables, which must be removed.                |
| `MIGRATE_LEGACY_DATA` | Set to `true` for the first start against the TS signer's Postgres database. |
| `CHAIN_ID`            | Set explicitly. The default is Celo Sepolia (`11142220`).                    |
| `LOG_FORMAT`          | Set to `json` or `text`. `stackdriver` and `human` stop the signer.          |
| `KEYSTORE_TYPE`       | Must be `GoogleSecretManager` (or `Mock` for development).                   |

`MIGRATE_LEGACY_DATA=true` imports the TS signer's `accountsOnChain` and `requestsOnChain` tables
into the Rust signer's own tables, once. Without it the Rust signer refuses to start on a database
that holds those tables, because every account's used quota would read zero. Once the import has
run, the variable can be removed. The import does not pick up what a TS signer writes afterwards, and
the TS signer does not see what the Rust signer records, so stop the TS signer before the first start
and do not switch back without accepting that loss.

Request rows without a stored signature, which the TS signer wrote before August 2023, are not
imported. The import logs their number as `requests_skipped_null_signature`. A replay of such a
request is charged as a new one. The TS signer prunes requests older than 7 days by default, so the
number is normally 0.

The TS signer kept one row for each spelling of an address, so an account that sent both
`0xabc…` and `0xABC…` has its used quota split over two rows. The import stores every address in
checksummed form and adds the counts together. The sum is the true number of charged queries, so
such an account can show a higher used quota after the switch than a TS signer reported.

The TS image ships a `.env` file that sets `CHAIN_ID=42220`. The Rust image ships none, so a mainnet
manifest that never set `CHAIN_ID` must set it now. The chain ID selects the contract addresses for
keys and quota. At startup the signer asks the full node for its chain ID and refuses to start if it
differs from `CHAIN_ID`, or if the full node does not answer.

`DATABASE_URL` without an `sslmode` parameter uses TLS only if the server offers it and does not
verify the certificate. Add `?sslmode=verify-full` to get what `DB_USE_SSL=true` gave the TS signer.

These TS variables are not read. Replace the ones you set:

| TS variable                                                             | Rust signer                                                                            |
| ----------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| `RETRY_COUNT`                                                           | `FULL_NODE_RETRY_COUNT`: retries after the first try, where TS counted all tries       |
| `RETRY_DELAY_IN_MS`                                                     | `FULL_NODE_RETRY_DELAY_MS`                                                             |
| `TIMEOUT_MS`                                                            | `FULL_NODE_TIMEOUT_MS`, same default of 1000                                           |
| `REQUEST_PRUNNING_DAYS`                                                 | `REQUEST_PRUNING_DAYS`                                                                 |
| `REQUEST_PRUNNING_AT_SERVER_START`, `REQUEST_PRUNNING_JOB_CRON_PATTERN` | `REQUEST_PRUNING_INTERVAL_SECS`: prunes at every start, then on this interval          |
| `SERVER_SSL_KEY_PATH`, `SERVER_SSL_CERT_PATH`                           | None. The signer serves plain HTTP; terminate TLS in front of it.                      |
| `DOMAINS_API_ENABLED` and the `DOMAINS_*` key variables                 | None. The domain endpoints return 503.                                                 |
| `KEYSTORE_AZURE_*`, `KEYSTORE_AWS_*`                                    | None. Only Google Secret Manager is supported.                                         |
| `SHOULD_MOCK_ACCOUNT_SERVICE`, `SHOULD_MOCK_REQUEST_SERVICE`            | None. There are no mock services to switch to; set to `true`, either stops the signer. |
| `TRACER_ENDPOINT`, `TRACING_SERVICE_NAME`                               | None. The signer exports no traces.                                                    |

`SERVER_PORT`, `PHONE_NUMBER_PRIVACY_API_ENABLED`, `PHONE_NUMBER_PRIVACY_KEY_NAME_BASE`,
`PHONE_NUMBER_PRIVACY_LATEST_KEY_VERSION`, `KEYSTORE_GOOGLE_PROJECT_ID`, `BLOCKCHAIN_PROVIDER`,
`BLOCKCHAIN_API_KEY`, `ODIS_SIGNER_TIMEOUT`, `DB_TIMEOUT` and `QUERY_PRICE_PER_CUSD` keep their names and
meaning.

Values are read more strictly than by the TS signer. A boolean must be `true` or `false`; the TS
signer read anything else, such as `1`, as false. A number or boolean set to an empty string stops
the signer, where the TS signer fell back to its default.

The signer does not close idle connections; the TS signer closed a socket after 6 seconds without
data. A signer that can be reached from outside its cluster needs a proxy or load balancer in front
that enforces an idle timeout.

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

Two error counters differ:

- `errors_caught_in_endpoint_handler` counts handler panics only. The TS signer also counted full
  node and database failures there; in the Rust signer those show in `blockchain_errors`,
  `database_errors` and `responses`.
- `errors_thrown_after_response_sent` does not exist. A Rust handler returns its response, so it
  cannot fail after sending one.

## Database configuration

The database is set with one variable, `DATABASE_URL` (`postgres://user:password@host:5432/database`
or `sqlite:path`). The TS signer's `DB_TYPE`, `DB_HOST`, `DB_PORT`, `DB_USERNAME`, `DB_PASSWORD`,
`DB_DATABASE` and `DB_USE_SSL` are not read.

`DB_TIMEOUT` is read and means what it meant in the TS signer: the limit in milliseconds for one
statement, default 1000. On Postgres the server enforces it, and it also limits the wait for a free
connection. On SQLite it is the wait for a lock. Pruning and the legacy import are not limited.

Three cases stop the signer at startup, so a manifest written for the TS signer cannot start it on an
empty database:

- `DB_HOST` or `DB_TYPE` is set and `DATABASE_URL` is not.
- The key store is not `Mock` and the database is in-memory SQLite, which is the default when
  neither `DATABASE_URL` nor `DB_PATH` is set.
- The Postgres database holds the TS signer's tables (`accountsOnChain` or `requestsOnChain`), they
  were never imported, and `MIGRATE_LEGACY_DATA` is not `true`.

## Replays under another key version

A query is charged once per account, whichever key versions sign it. The signer stores, with each
charged request, the signature and the key version that made it. A replay is answered from the
store only when the stored version is the one the replay resolves to, by its `odis-key-version`
header or by `PHONE_NUMBER_PRIVACY_LATEST_KEY_VERSION`. Otherwise the signer signs again with the
requested key and answers with the replay warning, without a second charge. If it does not hold
that key, it answers 500 with `CELO_ODIS_ERR_05`, as for a new request.

The TS signer answers every replay with the stored signature and names the requested version in the
`odis-key-version` header, so a replay under another version gets a partial signature made with the
wrong key. A combiner with both kinds of signers gets valid partial signatures for such a replay
from the Rust signers only.

The version is the nullable column `requests.key_version`. Rows imported from the TS signer have no
version, because the TS signer never recorded it. Their stored signature is never served: each
replay of such a request is signed again. The stored row is not rewritten, so this lasts until the
row is pruned (`REQUEST_PRUNING_DAYS`, default 7).

## Database failures

If the lookup for a replayed request fails, `/sign` answers 500 with `CELO_ODIS_ERR_03`. The TS
signer logged the failure and carried on as if the request were new.

## Request body size

`/sign` and `/quotaStatus` refuse a body of more than 16,000 characters with 400
`CELO_ODIS_WARN_01`, as the TS signer does. Characters are counted as JavaScript counts them, so a
body the combiner accepts is accepted here. The Rust signer counts the body as sent; the TS signer
counts it after parsing and writing it out again, so a body padded with whitespace can be refused
here and accepted there. The combiner forwards bodies without such padding. A body over 0.2 MB is
answered 413, as by the TS signer.
