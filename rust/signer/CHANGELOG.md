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
