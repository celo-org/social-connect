---
'@celo/identity': major
---

Remove the off-chain data module, the package's implementation of CIP-8 off-chain account data: signed files at a storage root named in an account's metadata, read and written through public and encrypted accessors. The package root did not export it, so only deep imports break. There is no replacement.

- Removed: the deep imports `@celo/identity/lib/offchain-data-wrapper`, `@celo/identity/lib/offchain/utils`, `@celo/identity/lib/offchain/storage-writers` and the seven under `@celo/identity/lib/offchain/accessors/`: `authorized-signer`, `binary`, `errors`, `interfaces`, `name`, `pictures` and `simple`.
- Removed: `OffchainDataWrapper` and its implementation `BasicDataWrapper`, the error enum `OffchainErrorTypes`, the error classes `FetchError`, `InvalidSignature`, `NoStorageRootProvidedData` and `NoStorageProvider`, and their union `OffchainErrors`.
- Removed: the accessors `AuthorizedSignerAccessor`, `PublicNameAccessor`, `PrivateNameAccessor`, `PublicPictureAccessor`, `PrivatePictureAccessor`, `PublicBinaryAccessor`, `PrivateBinaryAccessor`, `PublicSimpleAccessor` and `PrivateSimpleAccessor`, the interfaces `PublicAccessor` and `PrivateAccessor`, and the type `NameType`.
- Removed: the error enum `SchemaErrorTypes`, the error classes `InvalidDataError`, `OffchainError`, `UnknownCiphertext`, `UnavailableKey` and `InvalidKey`, and their union `SchemaErrors`.
- Removed: the storage writers `StorageWriter`, `LocalStorageWriter`, `GitStorageWriter`, `GoogleStorageWriter`, `AwsStorageWriter` and `MockStorageWriter`.
- Removed: the helpers `writeEncrypted`, `writeSymmetricKeys`, `readEncrypted`, `deserialize`, `buildEIP712TypedData`, `signBuffer` and `resolvePath`.

`@celo/contractkit` is not a peer dependency of `@celo/identity`, and the package does not depend on `@celo/metadata-claims` or `ethereum-cryptography`.
