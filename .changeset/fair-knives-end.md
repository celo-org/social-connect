---
"@celo/phone-number-privacy-monitor": major
---

Replace @celo/contractkit with viem


### Breaking Changes 

`queryOdisForQuota` and `queryOdisForSalt` for first param instead of a string url now take an object with rpcURL and chainID. 

```diff
- queryOdisForQuota("https://forno.celo.org",...rest)
+ queryOdisForQuota({rpcURL: "https://forno.celo.org", chainID: 42220},...rest)


- queryOdisForSalt("https://forno.celo.org",...rest)
+ queryOdisForSalt({rpcURL: "https://forno.celo.org", chainID: 42220},...rest)
```