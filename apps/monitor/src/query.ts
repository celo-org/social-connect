import { generateKeys, generateMnemonic, MnemonicStrength } from '@celo/cryptographic-utils'
import {
  buildOdisDomain,
  OdisHardeningConfig,
  odisHardenKey,
  odisQueryAuthorizer,
} from '@celo/encrypted-backup'
import { OdisUtils } from '@celo/identity'
import {
  AuthSigner,
  getServiceContext,
  OdisAPI,
  OdisContextName,
} from '@celo/identity/lib/odis/query'
import { genSessionID } from '@celo/phone-number-privacy-common/lib/utils/logger'
import {
  ensureLeading0x,
  normalizeAddressWith0x,
  privateKeyToAddress,
} from '@celo/utils/lib/address'
import { defined } from '@celo/utils/lib/sign-typed-data-utils'
import { defineString } from 'firebase-functions/params'
import { Account, Address, createWalletClient, extractChain, http } from 'viem'
import { privateKeyToAccount } from 'viem/accounts'
import { celo, celoAlfajores } from 'viem/chains'
import { dekAuthSigner, generateRandomPhoneNumber, PRIVATE_KEY } from './resources'

let phoneNumber: string

const defaultPhoneNumber = defineString('PHONE_NUMBER')
const newPrivateKey = async () => {
  const mnemonic = await generateMnemonic(MnemonicStrength.s256_24words)
  return (await generateKeys(mnemonic)).privateKey
}

type ChainInfo = {
  rpcURL: string
  chainID: 44787 | 42220
}

export const queryOdisForSalt = async (
  blockchainProvider: ChainInfo,
  contextName: OdisContextName,
  timeoutMs: number = 10000,
  bypassQuota: boolean = false,
  useDEK: boolean = false,
  privateKey?: string,
  privateKeyPercentage: number = 100,
) => {
  const serviceContext = getServiceContext(contextName, OdisAPI.PNP)

  const { accountAddress, authSigner } = await getAuthSignerAndAccount(
    blockchainProvider,
    useDEK,
    privateKey,
    privateKeyPercentage,
  )

  const abortController = new AbortController()
  const timeout = setTimeout(() => {
    abortController.abort()
    console.log(`ODIS salt request timed out after ${timeoutMs} ms`) // tslint:disable-line:no-console
  }, timeoutMs)
  try {
    const testSessionId = Math.floor(Math.random() * 100000).toString()
    const res = await OdisUtils.Identifier.getObfuscatedIdentifier(
      phoneNumber,
      OdisUtils.Identifier.IdentifierPrefix.PHONE_NUMBER,
      accountAddress,
      authSigner,
      serviceContext,
      undefined,
      undefined,
      undefined,
      bypassQuota ? testSessionId : genSessionID(),
      undefined,
      abortController,
    )
    clearTimeout(timeout)

    return res
  } catch (error) {
    clearTimeout(timeout)
    throw error
  }
}

export const queryOdisForQuota = async (
  blockchainProvider: ChainInfo,
  contextName: OdisContextName,
  timeoutMs: number = 10000,
  privateKey?: string,
  privateKeyPercentage: number = 100,
) => {
  console.log(`contextName: ${contextName}`) // tslint:disable-line:no-console
  console.log(`blockchain provider: ${blockchainProvider}`) // tslint:disable-line:no-console

  const serviceContext = getServiceContext(contextName, OdisAPI.PNP)

  if (!privateKey || Math.random() > privateKeyPercentage * 0.01) {
    privateKey = await newPrivateKey()
  }

  const account = privateKeyToAccount(ensureLeading0x(privateKey))

  const accountAddress = normalizeAddressWith0x(privateKeyToAddress(privateKey))

  const client = makeClient(blockchainProvider, account)
  const authSigner: AuthSigner = {
    authenticationMethod: OdisUtils.Query.AuthenticationMethod.WALLET_KEY,
    sign191: ({ message, account }) => client.signMessage({ message, account }),
  }

  const abortController = new AbortController()
  const timeout = setTimeout(() => {
    abortController.abort()
  }, timeoutMs)

  try {
    const res = await OdisUtils.Quota.getPnpQuotaStatus(
      accountAddress,
      authSigner,
      serviceContext,
      undefined,
      undefined,
      abortController,
    )

    clearTimeout(timeout)

    return res
  } catch (error) {
    clearTimeout(timeout)
    throw error
  }
}

export const queryOdisDomain = async (contextName: OdisContextName) => {
  console.log(`contextName: ${contextName}`)

  const serviceContext = getServiceContext(contextName, OdisAPI.DOMAIN)
  const monitorDomainConfig: OdisHardeningConfig = {
    rateLimit: [
      {
        delay: 0,
        resetTimer: defined(true),
        // Running every 5 min, this should not run out for the next 9 million years
        batchSize: defined(1000000000000),
        repetitions: defined(1000000000000),
      },
    ],
    environment: serviceContext,
  }
  const authorizer = odisQueryAuthorizer(Buffer.from('ODIS domains monitor authorizer test seed'))
  const domain = buildOdisDomain(monitorDomainConfig, authorizer.address)
  // Throws if signature verification fails
  return odisHardenKey(Buffer.from('password'), domain, serviceContext, authorizer.wallet)
}

async function getAuthSignerAndAccount(
  blockchainProvider: ChainInfo,
  useDEK: boolean,
  privateKey: string | undefined,
  privateKeyPercentage: number,
) {
  let authSigner: AuthSigner
  let accountAddress: Address

  if (useDEK) {
    // im not sure why this is like this
    if (!privateKey || Math.random() > privateKeyPercentage * 0.01) {
      privateKey = PRIVATE_KEY
    }
    accountAddress = normalizeAddressWith0x(privateKeyToAddress(privateKey)) as Address
    authSigner = dekAuthSigner(0)
    phoneNumber = generateRandomPhoneNumber()
  } else {
    // im not sure why this is like this.
    if (!privateKey || Math.random() > privateKeyPercentage * 0.01) {
      privateKey = await newPrivateKey()
    }
    accountAddress = normalizeAddressWith0x(privateKeyToAddress(privateKey)) as Address
    const account = privateKeyToAccount(ensureLeading0x(privateKey))

    const client = makeClient(blockchainProvider, account)

    authSigner = {
      authenticationMethod: OdisUtils.Query.AuthenticationMethod.WALLET_KEY,
      sign191: client.signMessage,
    }
    phoneNumber = defaultPhoneNumber.value()
  }
  return { accountAddress, authSigner, privateKey }
}

function makeClient(chainInfo: ChainInfo, account: Account) {
  return createWalletClient({
    account: account,
    chain: extractChain({ chains: [celoAlfajores, celo], id: chainInfo.chainID }),
    transport: http(chainInfo.rpcURL),
  })
}
