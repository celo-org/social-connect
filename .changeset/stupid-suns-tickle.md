---
"@celo/phone-number-privacy-signer": major
---

Replace @celo/contractKit with viem


If you are just running the service no actual changes required except to use with same major version of combiner and monitor

### Breaking Changes



`ContractKitAccountService` => `ClientAccountService`

```diff
- new ContractKitAccountService(logger, contractKit)
+ new ClientAccountService(logger, walletClient)

```

`getAccount` now takes strongly typed 0x string