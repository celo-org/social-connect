---
"@celo/phone-number-privacy-common": major
---

Replace @celo/contractkit with viem


### BREAKING CHANGES
Config changes 

`BlockchainConfig` gains a new required `chainID` property

`provider` which is an overloaded term is renamed to `rpcURL`

```diff
config: BlockchainConfig = {
  - provider: FORNO_URL
  + rpcURL: FORNOURL
  + chainID: 42220
}
```

`getContractKit` => `getWalletClient`


functions replaced in utils/authentication

`newContractKitFetcher` => `newDEKFetcher`

```diff
-  newContractKitFetcher(contractKit: ContractKit, ...)
+  newDEKFetcher(viemClient: Client, ...)
```

functions with with changed signatures

`getDataEncryptionKey`

```diff
export async function getDataEncryptionKey(
-  address: string,
-  contractKit: ContractKit,
+  address: Address,
+  viemClient: Client,
  logger: Logger,
  fullNodeTimeoutMs: number,
  fullNodeRetryCount: number,
  fullNodeRetryDelayMs: number,
- ): Promise<string>
+ ): Promise<Hex>
```

functions removed from test/utils

- `createMockToken`
- `createMockContractKit`
- `createMockConnection`
- `createMockWeb3`
- `replenishQuota`


### NEW FUNCTIONS

`import {getAccountsContract, getOdisPaymentsContract, getCUSDContract } from @celo/phone-number-privacy-common`

To replace contractKit wrappers for Accounts, OdisPayments, and StableToken, contracts. 
