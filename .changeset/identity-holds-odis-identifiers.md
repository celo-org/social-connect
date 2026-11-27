---
'@celo/identity': major
---

`@celo/identity` holds the identifier prefixes and the identifier hash, and does not depend on `@celo/odis-identifiers`, which gets no further releases. Import `IdentifierPrefix`, `getPrefixedIdentifier`, `getIdentifierHash` and `PEPPER_SEPARATOR` from `@celo/identity` as `OdisUtils.Identifier.<name>`, or from the deep path `@celo/identity/lib/odis/identifier`.

`getIdentifierHash` in `@celo/identity` takes `(plaintextIdentifier, identifierPrefix, pepper)` and supplies the hash function, `soliditySha3`, itself. The function of the same name in `@celo/odis-identifiers` takes the hash function as its first parameter.
