---
"@celo/phone-number-privacy-combiner": major
---

Replace @celo/contractkit with viem


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
---

`lib/common/web3` => `lib/common`

`getDEK` moved from 

it now takes a WalletClient as its first param instead of a ContractKit.

third param is now typed to require address starts with 0x 

```diff
- export async function getDEK(kit: ContractKit, logger: Logger, account: string): Promise<string> 
+ export async function getDEK(client: Client, logger: Logger, account: Address): Promise<string>
```

---

lib/pnp/services/account-services`

`ContractKitAccountServiceOptions` => `ViemAccountServiceOptions`

`ContractKitAccountService` => `ViemAccountService`

addressed passed to `getAccount` now MUST start with `0x` in type