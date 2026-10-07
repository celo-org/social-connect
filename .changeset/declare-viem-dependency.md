---
'@celo/phone-number-privacy-common': patch
'@celo/identity': patch
---

Declare `viem` as a dependency. Both packages import it at runtime, so installing them in a project without viem failed with `Cannot find module 'viem'`.
