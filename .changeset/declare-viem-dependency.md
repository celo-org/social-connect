---
'@celo/identity': patch
---

Declare `viem` as a dependency. The package imports it at runtime, so installing it in a project without viem failed with `Cannot find module 'viem'`.
