import { newKit, newKitWithApiKey } from '@celo/contractkit'

const kit = newKit('mainnet')
// offchain data wrapper
const accounts = await kit.contracts.getAccounts()
accounts.getMetadataURL(address)
// import { ClaimTypes } from '@celo/contractkit/lib/identity/claims/types'
import { IdentityMetadataWrapper } from '@celo/contractkit/lib/identity/metadata'
IdentityMetadataWrapper.fetchFromURL

/// packages/common/src/utils/contracts.ts
newKitWithApiKey
// newKit with the HttpProviderOptions

/// packages/common/src/utils/authentication.ts
const accountWrapper = await kit.contracts.getAccounts()
accountWrapper.getDataEncryptionKey(address)

// apps/signer/src/common/web3/contracts.ts
kit.contracts.getOdisPayments().totalPaidCUSD

// apps/monitor/src/query.ts
contractKit.connection.addAccount(privateKey)

accounts.generateProofOfKeyPossessionLocally
