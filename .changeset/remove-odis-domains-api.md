---
'@celo/identity': major
---

Remove the ODIS domains API and `OdisUtils.CircuitBreaker`. ODIS domains are deprecated, and `@celo/encrypted-backup`, their only client, is no longer maintained. There is no replacement.

- Removed: `sendOdisDomainRequest`, `ODIS_CELO_SEPOLIA_CONTEXT_DOMAINS`, `ODIS_MAINNET_CONTEXT_DOMAINS` and the enum `OdisAPI`. `getServiceContext` takes the context name only, and `getServiceContext(context)` answers as before; a caller that passes `OdisAPI.PNP` as a second argument drops it.
- Renamed: the service contexts are `ODIS_CELO_SEPOLIA_CONTEXT` and `ODIS_MAINNET_CONTEXT`, without the `_PNP` suffix that set them apart from the domain contexts. A consumer that imports `ODIS_CELO_SEPOLIA_CONTEXT_PNP` or `ODIS_MAINNET_CONTEXT_PNP` uses the name without the suffix.
- Removed: `OdisUtils.CircuitBreaker`.
- Removed: the domain requests and the endpoints `CombinerEndpoint.DOMAIN_SIGN`, `CombinerEndpoint.DOMAIN_QUOTA_STATUS` and `CombinerEndpoint.DISABLE_DOMAIN` from the types of `OdisUtils.Query.queryOdis`. The only requests it accepts are the PNP ones.
- Removed: the deep imports `@celo/identity/lib/odis/circuit-breaker` and `@celo/identity/lib/odis/circuit-breaker.mock`.
