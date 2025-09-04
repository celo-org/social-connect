---
"@celo/phone-number-privacy-common": major
---

Replaced all usage of @celo/contractkit with viem across the package.

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

---

You must update all initialization and contract access to the new Viem-based APIs and types.
