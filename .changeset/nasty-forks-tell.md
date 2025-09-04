---
"@celo/identity": major
---

Contract kit has been replaced with viem as dependency.

from `lib/odis/query`; WalletKeySigner instead of a contractKit instance now takes a sign191 function

- This should use EIP191 to sign the message using the private key assosiated with the account

Most places that were previously typed as string are now 0x-string typed

ContractKit is now an optional peer dependency. it is only needed if using the offchain-data-wrapper
