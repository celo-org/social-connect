# @celo/phone-number-privacy-combiner

## 3.3.2-beta.0

### Patch Changes

- 8f95181e1: Upgraded Dependencies https://github.com/celo-org/social-connect/pull/144
- 8638c3adf: Remove erroneos imports / dependent of @celo/phone-utils
- Updated dependencies [8f95181e1]
- Updated dependencies [8638c3adf]
- Updated dependencies [8638c3adf]
  - @celo/encrypted-backup@5.0.5-beta.0
  - @celo/identity@5.1.1-beta.0
  - @celo/phone-number-privacy-common@3.1.1-beta.0

## 3.3.1

### Patch Changes

- a55409c: Include all metrics in new Prometheus register

## 3.3.0

### Minor Changes

- f9fcf0e3d: Update Combiner to run as a daemon and add prometheus metrics. Add /metrics endpoint to CombinerEndpoints in common pkg. Small edits to Signer to fix integration tests now that both services use Prometheus metrics.

### Patch Changes

- Updated dependencies [f9fcf0e3d]
  - @celo/phone-number-privacy-common@3.1.0

## 3.2.1

### Patch Changes

- a66c122: Updated the tracing endpoint URL.

## 3.2.0

### Minor Changes

- ffe645c: Migrated the combiner from gen1 to gen2 cloud function. This changeset overwride the previous one.

### Patch Changes

- bf1ffb5: Migrated the combiner to gen2 cloud function.

## 3.1.0

### Minor Changes

- 27b3ee6: Added a proxy functionality to the gen1 combiner, allowing it to forward any requests received to the gen2 combiner

### Patch Changes

- baee530: Removed performance observer metric for combiner endpoint latency.
