# @celo/phone-number-privacy-common

## 4.0.0-beta.0

### Major Changes

- cc5df33: Replaced all usage of @celo/contractkit with viem across the package.

  ### BREAKING CHANGES

  - Config changes:

    - `BlockchainConfig` now requires a new property: `chainID`
    - `provider` renamed to `rpcURL`
      ```diff
      config: BlockchainConfig = {
        - provider: FORNO_URL
        + rpcURL: FORNO_URL
        + chainID: 42220
      }
      ```

  - Initialization changes:

    - `getContractKit` → `getWalletClient`
    - `getContractKitWithAgent` → `getWalletClientWithAgent`

  - Authentication and DEK fetching:

    - `newContractKitFetcher` → `newDEKFetcher`
      ```diff
      - newContractKitFetcher(contractKit: ContractKit, ...)
      + newDEKFetcher(viemClient: Client, ...)
      ```
    - `getDataEncryptionKey` signature changed:
      ```diff
      export async function getDataEncryptionKey(
        - address: string,
        - contractKit: ContractKit,
        + address: Address,
        + viemClient: Client,
        logger: Logger,
        fullNodeTimeoutMs: number,
        fullNodeRetryCount: number,
        fullNodeRetryDelayMs: number,
        - ): Promise<string>
        + ): Promise<Hex>
      ```

  - Functions removed from test/utils:
    - `createMockToken`
    - `createMockContractKit`
    - `createMockConnection`
    - `createMockWeb3`
    - `replenishQuota`
  - All addresses now typed as `Address` from viem.

  ### NEW EXPORTS

  - `getAccountsContract`, `getOdisPaymentsContract`, `getCUSDContract`:
    - These helpers replace contractKit wrappers and use viem + @celo/abis.

  ***

  You must update all initialization and contract access to the new Viem-based APIs and types.

### Minor Changes

- 49922d5: Add Celo Sepolia testnet support and fix E2E tests

  - Add Celo Sepolia testnet support across ODIS components including combiner, signer, monitor, and identity packages
  - Add Celo Sepolia contract addresses and RPC endpoints
  - Add test commands for running E2E tests against Celo Sepolia
  - Upgrade TypeScript to 5.4.5 in signer package to support @tsconfig/node22
  - Fix type assertion issues in E2E tests by using proper `as Type` syntax
  - Standardize DEK test values in common package to prevent conflicts between combiner and signer E2E tests
  - Both test suites now reference the same DEK values to avoid overriding each other on shared blockchain

## 3.1.2

### Patch Changes

- 8804b67: Upgrade @celo/\*\* dependencies to latest release
- 687607a: Remove opentelemetry dependencies
- 3d18f26: Update package dependencies to reduce CVEs

## 3.1.2-beta.0

### Patch Changes

- 8804b67: Upgrade @celo/\*\* dependencies to latest release
- 687607a: Remove opentelemetry dependencies
- 3d18f26: Update package dependencies to reduce CVEs

## 3.1.1

### Patch Changes

- 8f95181: Upgraded Dependencies https://github.com/celo-org/social-connect/pull/144
- 8638c3a: Remove erroneos imports / dependent of @celo/phone-utils

## 3.1.0

### Minor Changes

- f9fcf0e3d: Update Combiner to run as a daemon and add prometheus metrics. Add /metrics endpoint to CombinerEndpoints in common pkg. Small edits to Signer to fix integration tests now that both services use Prometheus metrics.
