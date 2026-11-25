---
'@celo/identity': patch
---

The offchain data wrapper encrypts with `@celo/utils` 8 instead of 6. The two versions derive ECIES keys differently when the shared secret starts with a zero byte, so a reader whose wallet uses `@celo/utils` 7 or later, such as `@celo/wallet-local` 6, failed with `MAC mismatch` on about 1 in 256 keys that `@celo/identity` encrypted to it. Such readers now decrypt every new key. Readers on `@celo/utils` 6 or older now fail on about 1 in 256 new keys instead, and keys written before this release that failed still fail until `@celo/utils` can read them.
