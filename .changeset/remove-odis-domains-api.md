---
'@celo/identity': major
---

Remove the ODIS domains API and `OdisUtils.CircuitBreaker`. ODIS domains are deprecated, and `@celo/encrypted-backup`, their only client, is no longer maintained. There is no replacement.

- Removed: `sendOdisDomainRequest`, `ODIS_CELO_SEPOLIA_CONTEXT_DOMAINS`, `ODIS_MAINNET_CONTEXT_DOMAINS` and `OdisAPI.DOMAIN`. `getServiceContext(context)` and `getServiceContext(context, OdisAPI.PNP)` work as before.
- Removed: `OdisUtils.CircuitBreaker`.
- Removed: the domain requests and the endpoints `CombinerEndpoint.DOMAIN_SIGN`, `CombinerEndpoint.DOMAIN_QUOTA_STATUS` and `CombinerEndpoint.DISABLE_DOMAIN` from the types of `OdisUtils.Query.queryOdis`. It accepts only PNP requests and endpoints.
- Removed: the deep imports `@celo/identity/lib/odis/circuit-breaker` and `@celo/identity/lib/odis/circuit-breaker.mock`.
