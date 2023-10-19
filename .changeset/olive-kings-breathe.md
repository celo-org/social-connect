---
'@celo/phone-number-privacy-combiner': minor
'@celo/phone-number-privacy-common': minor
'@celo/phone-number-privacy-signer': minor
---

Update Combiner to run as a daemon and add prometheus metrics. Add /metrics endpoint to CombinerEndpoints in common pkg. Small edits to Signer to fix integration tests now that both services use Prometheus metrics.
