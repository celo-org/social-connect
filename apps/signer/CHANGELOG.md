# @celo/phone-number-privacy-signer

## 4.0.0-beta.0

### Major Changes

- cc5df33: Replace @celo/contractKit with viem

  If you are just running the service no actual changes required except to use with same major version of combiner and monitor

  ### Breaking Changes

  `ContractKitAccountService` => `ClientAccountService`

  ```diff
  - new ContractKitAccountService(logger, contractKit)
  + new ClientAccountService(logger, walletClient)

  ```

  `getAccount` now takes strongly typed 0x string

### Patch Changes

- 49922d5: Add Celo Sepolia testnet support and fix E2E tests
  - Add Celo Sepolia testnet support across ODIS components including combiner, signer, monitor, and identity packages
  - Add Celo Sepolia contract addresses and RPC endpoints
  - Add test commands for running E2E tests against Celo Sepolia
  - Upgrade TypeScript to 5.4.5 in signer package to support @tsconfig/node22
  - Fix type assertion issues in E2E tests by using proper `as Type` syntax
  - Standardize DEK test values in common package to prevent conflicts between combiner and signer E2E tests
  - Both test suites now reference the same DEK values to avoid overriding each other on shared blockchain

- Updated dependencies [49922d5]
- Updated dependencies [cc5df33]
  - @celo/phone-number-privacy-common@4.0.0-beta.0

## 3.1.2

### Patch Changes

- 14269eb: Updated the e2e test script.
- 8804b67: Upgrade @celo/\*\* dependencies to latest release
- 3d18f26: Update package dependencies to reduce CVEs
- Updated dependencies [8804b67]
- Updated dependencies [687607a]
- Updated dependencies [3d18f26]
  - @celo/phone-number-privacy-common@3.1.2

## 3.1.2-beta.0

### Patch Changes

- 14269eb: Updated the e2e test script.
- 8804b67: Upgrade @celo/\*\* dependencies to latest release
- 3d18f26: Update package dependencies to reduce CVEs
- Updated dependencies [8804b67]
- Updated dependencies [687607a]
- Updated dependencies [3d18f26]
  - @celo/phone-number-privacy-common@3.1.2-beta.0

## 3.1.1

### Patch Changes

- 8f95181: Upgraded Dependencies https://github.com/celo-org/social-connect/pull/144
- Updated dependencies [8f95181]
- Updated dependencies [8638c3a]
  - @celo/phone-number-privacy-common@3.1.1

## 3.1.0

### Minor Changes

- f9fcf0e3d: Update Combiner to run as a daemon and add prometheus metrics. Add /metrics endpoint to CombinerEndpoints in common pkg. Small edits to Signer to fix integration tests now that both services use Prometheus metrics.

### Patch Changes

- Updated dependencies [f9fcf0e3d]
  - @celo/phone-number-privacy-common@3.1.0
