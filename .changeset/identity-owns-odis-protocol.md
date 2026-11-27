---
'@celo/identity': major
---

`@celo/identity` owns the ODIS protocol types and does not depend on `@celo/phone-number-privacy-common`, which gets no further releases. Import these from `@celo/identity`:

- `AuthenticationMethod`, `KEY_VERSION_HEADER`, `signWithRawKey` and `getMessageDigest`.
- The error strings: `ErrorMessage`, `WarningMessage` and their union `ErrorType`.
- The endpoint paths, in two enums: `SignerEndpoint` and `CombinerEndpoint`.
- The request and response types `OdisRequest`, `OdisRequestHeader` and `OdisResponse`, with `SignMessageRequest`, `PnpQuotaRequest`, their headers and response types, and the response schemas `SignMessageResponseSchema` and `PnpQuotaResponseSchema`.

Names from `@celo/phone-number-privacy-common` that have no counterpart in `@celo/identity`:

- `CombinerEndpointPNP`, `SignerEndpointPNP`, `CombinerEndpointCommon`, `SignerEndpointCommon` and `Endpoint`: use `CombinerEndpoint` or `SignerEndpoint`.
- `PhoneNumberPrivacyRequest` (which `@celo/identity/lib/odis/query` also exported in 5.x), `PhoneNumberPrivacyRequestHeader` and `PhoneNumberPrivacyResponse`: use `OdisRequest`, `OdisRequestHeader` and `OdisResponse`.
- `SuccessResponse`, `FailureResponse`, `getSignerEndpoint`, `getCombinerEndpoint`, `SignMessageRequestSchema` and `PnpQuotaRequestSchema` (the services validated incoming requests with these two): no replacement.
- The domain types, `DomainEndpoint` and the POPRF classes, which belong to the ODIS domains API: no replacement.
- The rest of the package (the logger, environment helpers, request authentication and validation, key-version handling, the `send` response helper, service constants, wallet-client and contract getters, and test fixtures) was code for the ODIS services and their tests, and has no counterpart.

An enum imported from `@celo/phone-number-privacy-common`, such as `AuthenticationMethod`, is a separate type from identity's: TypeScript accepts its values only while both enums have the same name and members, so take enums from `@celo/identity`.
