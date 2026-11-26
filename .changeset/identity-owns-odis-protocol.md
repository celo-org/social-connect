---
'@celo/identity': major
---

`@celo/identity` no longer depends on `@celo/phone-number-privacy-common`, which gets no further releases. The ODIS request and response types, endpoints, error strings, `KEY_VERSION_HEADER` and `signWithRawKey` are exported from `@celo/identity` itself. Import them from there; enums such as `AuthenticationMethod` from `@celo/phone-number-privacy-common` are no longer the same type as identity's. The endpoint paths are in two enums, `SignerEndpoint` and `CombinerEndpoint`; `CombinerEndpointPNP`, `SignerEndpointPNP`, `CombinerEndpointCommon`, `SignerEndpointCommon` and `Endpoint` from `@celo/phone-number-privacy-common` have no counterpart: use `CombinerEndpoint` or `SignerEndpoint`.
