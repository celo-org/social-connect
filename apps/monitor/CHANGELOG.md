# @celo/phone-number-privacy-monitor

## 4.0.0-beta.0

### Major Changes

- cc5df33: Replace @celo/contractkit with viem

  ### Breaking Changes

  `queryOdisForQuota` and `queryOdisForSalt` for first param instead of a string url now take an object with rpcURL and chainID.

  ```diff
  - queryOdisForQuota("https://forno.celo.org",...rest)
  + queryOdisForQuota({rpcURL: "https://forno.celo.org", chainID: 42220},...rest)


  - queryOdisForSalt("https://forno.celo.org",...rest)
  + queryOdisForSalt({rpcURL: "https://forno.celo.org", chainID: 42220},...rest)
  ```

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

## 3.1.0

### Minor Changes

- 89dc2a9: Updated to run on cloud function gen2 using node18. Updated test script to point to latest Clabs Signer deployments.

### Patch Changes

- 8804b67: Upgrade @celo/\*\* dependencies to latest release
- 14269eb: Added `ts-node` to monitor package
- 3d18f26: Update package dependencies to reduce CVEs
- Updated dependencies [8804b67]
- Updated dependencies [687607a]
- Updated dependencies [56164f9]
- Updated dependencies [3d18f26]
  - @celo/encrypted-backup@5.0.6
  - @celo/identity@5.1.2
  - @celo/phone-number-privacy-common@3.1.2

## 3.1.0-beta.0

### Minor Changes

- 89dc2a9: Updated to run on cloud function gen2 using node18. Updated test script to point to latest Clabs Signer deployments.

### Patch Changes

- 8804b67: Upgrade @celo/\*\* dependencies to latest release
- 14269eb: Added `ts-node` to monitor package
- 3d18f26: Update package dependencies to reduce CVEs
- Updated dependencies [8804b67]
- Updated dependencies [687607a]
- Updated dependencies [56164f9]
- Updated dependencies [3d18f26]
  - @celo/encrypted-backup@5.0.6-beta.0
  - @celo/identity@5.1.2-beta.0
  - @celo/phone-number-privacy-common@3.1.2-beta.0

## 3.0.2

### Patch Changes

- 8f95181: Upgraded Dependencies https://github.com/celo-org/social-connect/pull/144
- Updated dependencies [8f95181]
- Updated dependencies [8638c3a]
- Updated dependencies [8638c3a]
  - @celo/encrypted-backup@5.0.5
  - @celo/identity@5.1.1
  - @celo/phone-number-privacy-common@3.1.1
