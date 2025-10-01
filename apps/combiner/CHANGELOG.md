# @celo/phone-number-privacy-combiner

## 4.0.0-beta.0

### Major Changes

- cc5df33: Replace @celo/contractkit with viem

  ### Breaking Changes

  `startCombiner` now takes an optional WalletClient instance from viem instead of a ContractKit instance

  config passed to `startCombiner` has a sub config `blockchain` which has the following changes.

  ```diff
  {
    ...restOfConfig,
    blockchain: {
      - provider: "https://forno.celo.org"
      + rpcURL: "https://forno.celo.org"
      + chainID: 42220
    }
  }

  ```

  ***

  `lib/common/web3` => `lib/common`

  `getDEK` moved from

  it now takes a WalletClient as its first param instead of a ContractKit.

  third param is now typed to require address starts with 0x

  ```diff
  - export async function getDEK(kit: ContractKit, logger: Logger, account: string): Promise<string>
  + export async function getDEK(client: Client, logger: Logger, account: Address): Promise<string>
  ```

  ***

  lib/pnp/services/account-services`

  `ContractKitAccountServiceOptions` => `ViemAccountServiceOptions`

  `ContractKitAccountService` => `ViemAccountService`

  addressed passed to `getAccount` now MUST start with `0x` in type

### Minor Changes

- 49922d5: Add Celo Sepolia testnet support and fix E2E tests
  - Add Celo Sepolia testnet support across ODIS components including combiner, signer, monitor, and identity packages
  - Add Celo Sepolia contract addresses and RPC endpoints
  - Add test commands for running E2E tests against Celo Sepolia
  - Upgrade TypeScript to 5.4.5 in signer package to support @tsconfig/node22
  - Fix type assertion issues in E2E tests by using proper `as Type` syntax
  - Standardize DEK test values in common package to prevent conflicts between combiner and signer E2E tests
  - Both test suites now reference the same DEK values to avoid overriding each other on shared blockchain

### Patch Changes

- Updated dependencies [49922d5]
- Updated dependencies [cc5df33]
- Updated dependencies [6dada95]
- Updated dependencies [cc5df33]
  - @celo/identity@6.0.0-beta.0
  - @celo/phone-number-privacy-common@4.0.0-beta.0
  - @celo/encrypted-backup@5.0.7-beta.0

## 3.3.3

### Patch Changes

- 8804b67: Upgrade @celo/\*\* dependencies to latest release
- 3d18f26: Update package dependencies to reduce CVEs
- Updated dependencies [8804b67]
- Updated dependencies [687607a]
- Updated dependencies [56164f9]
- Updated dependencies [3d18f26]
  - @celo/encrypted-backup@5.0.6
  - @celo/identity@5.1.2
  - @celo/phone-number-privacy-common@3.1.2

## 3.3.3-beta.0

### Patch Changes

- 8804b67: Upgrade @celo/\*\* dependencies to latest release
- 3d18f26: Update package dependencies to reduce CVEs
- Updated dependencies [8804b67]
- Updated dependencies [687607a]
- Updated dependencies [56164f9]
- Updated dependencies [3d18f26]
  - @celo/encrypted-backup@5.0.6-beta.0
  - @celo/identity@5.1.2-beta.0
  - @celo/phone-number-privacy-common@3.1.2-beta.0

## 3.3.2

### Patch Changes

- 8f95181: Upgraded Dependencies https://github.com/celo-org/social-connect/pull/144
- 8638c3a: Remove erroneos imports / dependent of @celo/phone-utils
- Updated dependencies [8f95181]
- Updated dependencies [8638c3a]
- Updated dependencies [8638c3a]
  - @celo/encrypted-backup@5.0.5
  - @celo/identity@5.1.1
  - @celo/phone-number-privacy-common@3.1.1

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
