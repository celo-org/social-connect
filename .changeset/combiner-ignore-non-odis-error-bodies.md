---
"@celo/phone-number-privacy-combiner": patch
---

Ignore signer error responses that are not ODIS responses (e.g. reverse proxy error pages). Previously such a body could be read as a domain state and make `/domain/quotaStatus`, `/domain/sign` or `/domain/disable` fail.
