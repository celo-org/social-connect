import {
  EncryptionKeySigner,
  OdisContextName,
  WalletKeySigner,
} from '@celo/identity/lib/odis/query'
import { AuthenticationMethod } from '@celo/phone-number-privacy-common'
import { DEK_PRIVATE_KEY, DEK_PUBLIC_KEY } from '@celo/phone-number-privacy-common/lib/test/values'
import {
  ensureLeading0x,
  normalizeAddressWith0x,
  privateKeyToAddress,
} from '@celo/utils/lib/address'
import { Address, createWalletClient, http } from 'viem'
import { privateKeyToAccount } from 'viem/accounts'
import { celo, celoSepolia } from 'viem/chains'
import { FORNO_CELO_SEPOLIA } from '../../src'

require('dotenv').config()

export const getTestContextName = (): OdisContextName => {
  switch (process.env.CONTEXT_NAME) {
    case 'staging':
      return OdisContextName.STAGING
    case 'mainnet':
      return OdisContextName.MAINNET
    case 'celo-sepolia':
      return OdisContextName.CELO_SEPOLIA
    default:
      throw new Error('CONTEXT_NAME env var is undefined or invalid')
  }
}

/**
 * CONSTS
 */
export const DEFAULT_FORNO_URL = process.env.ODIS_BLOCKCHAIN_PROVIDER ?? FORNO_CELO_SEPOLIA

export const PRIVATE_KEY = '2c63bf6d60b16c8afa13e1069dbe92fef337c23855fff8b27732b3e9c6e7efd4' // INFO: only valid for staging and celo sepolia
export const ACCOUNT_ADDRESS = normalizeAddressWith0x(privateKeyToAddress(PRIVATE_KEY)) as Address // 0x6037800e91eaa703e38bad40c01410bbdf0fea7e

// export const PRIVATE_KEY_NO_QUOTA =
// '2c63bf6d60b16c8afa13e1069dbe92fef337c23855fff8b27732b3e9c6e7efd4' // XXX use this PK on mainnet
export const PRIVATE_KEY_NO_QUOTA =
  '0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890000000'
export const ACCOUNT_ADDRESS_NO_QUOTA = privateKeyToAddress(PRIVATE_KEY_NO_QUOTA) as Address

export const PHONE_NUMBER = '+17777777777'
export const BLINDING_FACTOR = Buffer.from('0IsBvRfkBrkKCIW6HV0/T1zrzjQSe8wRyU3PKojCnww=', 'base64')
// BLINDED_PHONE_NUMBER value is dependent on PHONE_NUMBER AND BLINDING_FACTOR
// hardcoding to avoid importing blind_threshols_bls library
export const BLINDED_PHONE_NUMBER =
  'hZXDhpC5onzBSFa1agZ9vfHzqwJ/QeJg77NGvWiQG/sFWsvHETzZvdWr2GpF3QkB'

export const CONTACT_PHONE_NUMBER = '+14155559999'
export const CONTACT_PHONE_NUMBERS = [CONTACT_PHONE_NUMBER]

/**
 * RESOURCES AND UTILS
 */
const getViemChain = () => {
  const contextName = getTestContextName()
  switch (contextName) {
    case OdisContextName.MAINNET:
      return celo
    case OdisContextName.CELO_SEPOLIA:
      return celoSepolia
    case OdisContextName.STAGING:
      return celoSepolia
    default:
      break
  }
}

export const client = createWalletClient({
  transport: http(DEFAULT_FORNO_URL),
  chain: getViemChain(),
  account: privateKeyToAccount(ensureLeading0x(PRIVATE_KEY)),
})

interface DEK {
  privateKey: string
  publicKey: string
  address: string
}

export const deks: DEK[] = [
  {
    privateKey: DEK_PRIVATE_KEY.slice(2), // Remove 0x prefix
    publicKey: DEK_PUBLIC_KEY.slice(2), // Remove 0x prefix
    address: '0x7b33dF2607b85e3211738a49A6Ad6E8Ed4d13F6E',
  },
  {
    privateKey: '0975b0c565abc75b6638a749ea3008cb52676af3eabe4b80e19c516d82330364',
    publicKey: '03b1ac8c445f0796978018c087b97e8213b32c39e6a8642ae63dce71da33a19f65',
    address: '0x34332049B07Fab9a2e843A7C8991469d93cF6Ae6',
  },
]
// The following code can be used to generate more test DEKs
// const generateDEKs = (n: number): Promise<DEK[]> => Promise.all([...Array(n).keys()].map(
//   async () => await deriveDek(await generateMnemonic())
// ))

export const dekAuthSigner = (index: number): EncryptionKeySigner => {
  return {
    authenticationMethod: AuthenticationMethod.ENCRYPTION_KEY,
    rawKey: ensureLeading0x(deks[index].privateKey),
  }
}

export const walletAuthSigner: WalletKeySigner = {
  authenticationMethod: AuthenticationMethod.WALLET_KEY,
  sign191: (args) => {
    // Use the appropriate private key based on the account address
    const privateKey =
      args.account === ACCOUNT_ADDRESS_NO_QUOTA ? PRIVATE_KEY_NO_QUOTA : PRIVATE_KEY
    return privateKeyToAccount(ensureLeading0x(privateKey)).signMessage(args)
  },
}
