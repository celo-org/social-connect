# @celo/identity

## 6.0.0

### Major Changes

- cc5df33: Contract kit has been replaced with viem as dependency.

  from `lib/odis/query`; WalletKeySigner instead of a contractKit instance now takes a sign191 function
  - This should use EIP191 to sign the message using the private key assosiated with the account

  Most places that were previously typed as string are now 0x-string typed

  ContractKit is now an optional peer dependency. it is only needed if using the offchain-data-wrapper

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
  - @celo/phone-number-privacy-common@4.0.0

## 5.1.2

### Patch Changes

- 8804b67: Upgrade @celo/\*\* dependencies to latest release
- 56164f9: Updated devchain to support CELO CR11
- 3d18f26: Update package dependencies to reduce CVEs
- Updated dependencies [8804b67]
- Updated dependencies [687607a]
- Updated dependencies [3d18f26]
  - @celo/phone-number-privacy-common@3.1.2
  - @celo/odis-identifiers@1.0.1

## 5.1.2-beta.0

### Patch Changes

- 8804b67: Upgrade @celo/\*\* dependencies to latest release
- 56164f9: Updated devchain to support CELO CR11
- 3d18f26: Update package dependencies to reduce CVEs
- Updated dependencies [8804b67]
- Updated dependencies [687607a]
- Updated dependencies [3d18f26]
  - @celo/phone-number-privacy-common@3.1.2-beta.0
  - @celo/odis-identifiers@1.0.1-beta.0

## 5.1.1

### Patch Changes

- 8f95181: Upgraded Dependencies https://github.com/celo-org/social-connect/pull/144
- 8638c3a: Upgrade cross-fetch dep
- Updated dependencies [8f95181]
- Updated dependencies [8638c3a]
  - @celo/phone-number-privacy-common@3.1.1

## 5.1.0

### Minor Changes

- ad7cced: Updated the ODIS URL in the identity package. URL now points to combinerGen2.
- dbd246f: Update Combiner URLs in Identity SDK to point to K8s instead of Cloud Functions

### Patch Changes

- Updated dependencies [757b590]
- Updated dependencies [757b590]
  - @celo/odis-identifiers@1.0.0
