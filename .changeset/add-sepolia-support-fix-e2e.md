---
"@celo/identity": minor
"@celo/phone-number-privacy-common": minor
"@celo/phone-number-privacy-combiner": minor
"@celo/phone-number-privacy-monitor": minor
"@celo/phone-number-privacy-signer": patch
---

Add Sepolia testnet support and fix E2E tests

- Add Sepolia testnet support across ODIS components including combiner, signer, monitor, and identity packages
- Add Sepolia contract addresses and RPC endpoints
- Add test commands for running E2E tests against Sepolia
- Upgrade TypeScript to 5.4.5 in signer package to support @tsconfig/node22
- Fix type assertion issues in E2E tests by using proper `as Type` syntax
- Standardize DEK test values in common package to prevent conflicts between combiner and signer E2E tests
- Both test suites now reference the same DEK values to avoid overriding each other on shared blockchain