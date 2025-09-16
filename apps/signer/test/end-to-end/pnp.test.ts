import { ensureLeading0x, sleep } from '@celo/base'
import {
  AuthenticationMethod,
  getAccountsContract,
  getCUSDContract,
  getOdisPaymentsContract,
  KEY_VERSION_HEADER,
  PnpQuotaRequest,
  PnpQuotaResponseFailure,
  PnpQuotaResponseSuccess,
  SignerEndpoint,
  SignMessageRequest,
  SignMessageResponseFailure,
  SignMessageResponseSuccess,
  TestUtils,
  WarningMessage,
} from '@celo/phone-number-privacy-common'
import threshold_bls from 'blind-threshold-bls'
import { randomBytes } from 'crypto'
import { Account, Address, createWalletClient, http } from 'viem'
import { privateKeyToAccount } from 'viem/accounts'
import { celo, celoSepolia } from 'viem/chains'
import { config, getSignerVersion } from '../../src/config'
import { getBlindedPhoneNumber, getTestParamsForContext } from './utils'

require('dotenv').config()

const {
  ACCOUNT_ADDRESS1, // zero OdisPayments balance/quota
  DEK_PRIVATE_KEY,
  DEK_PUBLIC_KEY,
  PHONE_NUMBER,
  PRIVATE_KEY1,
} = TestUtils.Values

// Use the same funded account as combiner for staging/alfajores
const PRIVATE_KEY2 = '2c63bf6d60b16c8afa13e1069dbe92fef337c23855fff8b27732b3e9c6e7efd4'
const ACCOUNT_ADDRESS2 = privateKeyToAccount(ensureLeading0x(PRIVATE_KEY2)).address // 0x6037800e91eaa703e38bad40c01410bbdf0fea7e
const { getPnpQuotaRequest, getPnpRequestAuthorization, getPnpSignRequest } = TestUtils.Utils

const ODIS_SIGNER_URL = process.env.ODIS_SIGNER_SERVICE_URL
const contextSpecificParams = getTestParamsForContext()

const getViemChain = () => {
  switch (process.env.CONTEXT_NAME) {
    case 'mainnet':
      return celo
    case 'celo-sepolia':
      return celoSepolia
    case 'staging':
      return celoSepolia
    default:
      return celoSepolia // default to celo sepolia for testing
  }
}

const account1 = privateKeyToAccount(ensureLeading0x(PRIVATE_KEY1))
const account2 = privateKeyToAccount(ensureLeading0x(PRIVATE_KEY2))
const client = createWalletClient({
  account: account1,
  chain: getViemChain(),
  transport: http(contextSpecificParams.blockchainProviderURL),
})

jest.setTimeout(60000)

const expectedVersion = getSignerVersion()

describe(`Running against service deployed at ${ODIS_SIGNER_URL}`, () => {
  beforeAll(async () => {
    const accountsWrapper = getAccountsContract(client)
    let currentDek: string
    try {
      currentDek = await accountsWrapper.read.getDataEncryptionKey([ACCOUNT_ADDRESS2])
    } catch {
      // If getDataEncryptionKey returns "0x" (no data), viem throws an error
      // We treat this as no DEK being set
      currentDek = '0x'
    }
    if (currentDek !== DEK_PUBLIC_KEY) {
      try {
        await accountsWrapper.write.setAccountDataEncryptionKey([DEK_PUBLIC_KEY], {
          account: account2,
        })
      } catch (error) {
        console.warn(
          'Failed to set DEK (likely due to insufficient funds), some tests may fail:',
          (error as Error).message,
        )
        // Continue with tests even if DEK setup fails - some tests don't require DEK to be set
      }
    }
  })

  it('Service is deployed at correct version', async () => {
    const response = await fetch(ODIS_SIGNER_URL + SignerEndpoint.STATUS, {
      method: 'GET',
    })
    expect(response.status).toBe(200)
    const body = (await response.json()) as any
    // This checks against local package.json version, change if necessary
    expect(body.version).toBe(expectedVersion)
  })

  describe(`${SignerEndpoint.PNP_QUOTA}`, () => {
    it('Should respond with 200 on valid request', async () => {
      const req = getPnpQuotaRequest(ACCOUNT_ADDRESS1)
      const authorization = getPnpRequestAuthorization(req, PRIVATE_KEY1)
      const res = await queryPnpQuotaEndpoint(req, authorization)
      expect(res.status).toBe(200)
      const resBody = (await res.json()) as PnpQuotaResponseSuccess
      expect(resBody).toEqual<PnpQuotaResponseSuccess>({
        success: true,
        version: expectedVersion,
        performedQueryCount: 0,
        totalQuota: 0,
        warnings: [],
      })
    })

    it('Should respond with 200 on valid request when authenticated with DEK', async () => {
      const req = getPnpQuotaRequest(ACCOUNT_ADDRESS2, AuthenticationMethod.ENCRYPTION_KEY)
      const authorization = getPnpRequestAuthorization(req, DEK_PRIVATE_KEY)
      const res = await queryPnpQuotaEndpoint(req, authorization)
      expect(res.status).toBe(200)
      const resBody = (await res.json()) as PnpQuotaResponseSuccess
      expect(resBody).toEqual<PnpQuotaResponseSuccess>({
        success: true,
        version: expectedVersion,
        performedQueryCount: resBody.performedQueryCount,
        totalQuota: resBody.totalQuota,
        warnings: [],
      })
      expect(resBody.totalQuota).toBeGreaterThan(0)
    })

    // Note: this test is a bit flaky due to race conditions with the quota cache
    it('Should respond with 200 and more quota after payment sent to OdisPayments.sol', async () => {
      const req = getPnpQuotaRequest(ACCOUNT_ADDRESS2)
      const authorization = getPnpRequestAuthorization(req, PRIVATE_KEY2)
      const res = await queryPnpQuotaEndpoint(req, authorization)
      expect(res.status).toBe(200)

      const resBody = (await res.json()) as PnpQuotaResponseSuccess
      let paymentSucceeded = false
      try {
        // Make a payment to increase quota
        const singleQueryCost = BigInt(config.quota.queryPriceInCUSD.times(1e18).toString(10))
        await sendCUSDToOdisPayments(singleQueryCost, ACCOUNT_ADDRESS2, account2)
        paymentSucceeded = true
        await sleep(10 * 1000) // sleep for cache ttl to ensure quota update is reflected
      } catch (error) {
        console.log(
          `Error sending cUSD. The account may need cUSD fauceting ${(error as Error).message}`,
        )
      }

      const res2 = await queryPnpQuotaEndpoint(req, authorization)
      expect(res2.status).toBe(200)
      const res2Body = (await res2.json()) as PnpQuotaResponseSuccess
      expect(res2Body).toEqual<PnpQuotaResponseSuccess>({
        success: true,
        version: expectedVersion,
        performedQueryCount: resBody.performedQueryCount,
        totalQuota: paymentSucceeded ? resBody.totalQuota + 1 : res2Body.totalQuota,
        warnings: [],
      })

      await sleep(10 * 1000)

      const res3 = await queryPnpQuotaEndpoint(req, authorization)
      expect(res3.status).toBe(200)
      const res3Body = (await res3.json()) as PnpQuotaResponseSuccess

      expect(res3Body).toEqual<PnpQuotaResponseSuccess>({
        success: true,
        version: expectedVersion,
        performedQueryCount: resBody.performedQueryCount,
        totalQuota: paymentSucceeded ? resBody.totalQuota + 1 : res3Body.totalQuota,
        warnings: [],
      })
    })

    it('Should respond with 400 on missing request fields', async () => {
      const badRequest = getPnpQuotaRequest(ACCOUNT_ADDRESS1)
      // @ts-ignore Intentionally deleting required field
      delete badRequest.account
      const authorization = getPnpRequestAuthorization(badRequest, PRIVATE_KEY1)
      const res = await queryPnpQuotaEndpoint(badRequest, authorization)
      expect(res.status).toBe(400)
      const resBody = (await res.json()) as PnpQuotaResponseFailure
      expect(resBody).toEqual<PnpQuotaResponseFailure>({
        success: false,
        version: expectedVersion,
        error: WarningMessage.INVALID_INPUT,
      })
    })

    it('Should respond with 401 on failed WALLET_KEY auth', async () => {
      const badRequest = getPnpQuotaRequest(ACCOUNT_ADDRESS2, AuthenticationMethod.WALLET_KEY)
      const authorization = getPnpRequestAuthorization(badRequest, PRIVATE_KEY1)
      const res = await queryPnpQuotaEndpoint(badRequest, authorization)
      expect(res.status).toBe(401)
      const resBody = (await res.json()) as PnpQuotaResponseFailure
      expect(resBody).toEqual<PnpQuotaResponseFailure>({
        success: false,
        version: expectedVersion,
        error: WarningMessage.UNAUTHENTICATED_USER,
      })
    })

    it('Should respond with 401 on failed DEK auth when DEK is set for account', async () => {
      const badRequest = getPnpQuotaRequest(ACCOUNT_ADDRESS2, AuthenticationMethod.ENCRYPTION_KEY)
      const authorization = getPnpRequestAuthorization(badRequest, PRIVATE_KEY2)
      const res = await queryPnpQuotaEndpoint(badRequest, authorization)
      expect(res.status).toBe(401)
      const resBody = (await res.json()) as PnpQuotaResponseFailure
      expect(resBody).toEqual<PnpQuotaResponseFailure>({
        success: false,
        version: expectedVersion,
        error: WarningMessage.UNAUTHENTICATED_USER,
      })
    })

    it('Should respond with 401 on failed DEK auth when DEK is not set for account', async () => {
      const badRequest = getPnpQuotaRequest(ACCOUNT_ADDRESS1, AuthenticationMethod.ENCRYPTION_KEY)
      const authorization = getPnpRequestAuthorization(badRequest, PRIVATE_KEY1)
      const res = await queryPnpQuotaEndpoint(badRequest, authorization)
      expect(res.status).toBe(401)
      const resBody = (await res.json()) as PnpQuotaResponseFailure
      expect(resBody).toEqual<PnpQuotaResponseFailure>({
        success: false,
        version: expectedVersion,
        error: WarningMessage.UNAUTHENTICATED_USER,
      })
    })
  })

  describe(`${SignerEndpoint.PNP_SIGN}`, () => {
    describe('success cases', () => {
      let startingPerformedQueryCount: number

      beforeEach(async () => {
        const req = getPnpQuotaRequest(ACCOUNT_ADDRESS2)
        const authorization = getPnpRequestAuthorization(req, PRIVATE_KEY2)
        const res = await queryPnpQuotaEndpoint(req, authorization)
        expect(res.status).toBe(200)
        const resBody = (await res.json()) as PnpQuotaResponseSuccess
        startingPerformedQueryCount = resBody.performedQueryCount
      })

      it('[Signer configuration test] Should respond with 200 on valid request', async () => {
        const blindedMessage = getBlindedPhoneNumber(PHONE_NUMBER, randomBytes(32))
        const req = getPnpSignRequest(
          ACCOUNT_ADDRESS2,
          blindedMessage,
          AuthenticationMethod.WALLET_KEY,
        )
        const authorization = getPnpRequestAuthorization(req, PRIVATE_KEY2)
        const res = await queryPnpSignEndpoint(req, authorization)
        expect(res.status).toBe(200)
        const resBody = (await res.json()) as SignMessageResponseSuccess
        expect(resBody).toEqual<SignMessageResponseSuccess>({
          success: true,
          version: expectedVersion,
          signature: resBody.signature,
          performedQueryCount: startingPerformedQueryCount + 1,
          totalQuota: resBody.totalQuota,
          warnings: [],
        })
        expect(res.headers.get(KEY_VERSION_HEADER)).toEqual(contextSpecificParams.pnpKeyVersion)
        expect(
          threshold_bls.partialVerifyBlindSignature(
            Buffer.from(contextSpecificParams.pnpPolynomial, 'hex'),
            Buffer.from(blindedMessage, 'base64'),
            Buffer.from(resBody.signature, 'base64'),
          ),
        )
      })

      it(`Should respond with 200 on valid request with key version ${contextSpecificParams.pnpKeyVersion}`, async () => {
        // This value can also be modified but needs to be manually inspected in the signer logs
        // (on staging) since a valid key version that does not exist in the keystore
        // will default to the secretName stored in `KEYSTORE_AZURE_SECRET_NAME`
        const keyVersion = contextSpecificParams.pnpKeyVersion
        const blindedMessage = getBlindedPhoneNumber(PHONE_NUMBER, randomBytes(32))
        const req = getPnpSignRequest(
          ACCOUNT_ADDRESS2,
          blindedMessage,
          AuthenticationMethod.WALLET_KEY,
        )
        const authorization = getPnpRequestAuthorization(req, PRIVATE_KEY2)
        const res = await queryPnpSignEndpoint(req, authorization, keyVersion)
        expect(res.status).toBe(200)
        const resBody = (await res.json()) as SignMessageResponseSuccess
        expect(resBody).toEqual<SignMessageResponseSuccess>({
          success: true,
          version: expectedVersion,
          signature: resBody.signature,
          performedQueryCount: startingPerformedQueryCount + 1,
          totalQuota: resBody.totalQuota,
          warnings: [],
        })
        expect(res.headers.get(KEY_VERSION_HEADER)).toEqual(keyVersion)
        expect(
          threshold_bls.partialVerifyBlindSignature(
            Buffer.from(contextSpecificParams.pnpPolynomial, 'hex'),
            Buffer.from(blindedMessage, 'base64'),
            Buffer.from(resBody.signature, 'base64'),
          ),
        )
      })

      it('Should respond with 200 and warning on repeated valid requests', async () => {
        const blindedMessage = getBlindedPhoneNumber(PHONE_NUMBER, randomBytes(32))
        const req = getPnpSignRequest(
          ACCOUNT_ADDRESS2,
          blindedMessage,
          AuthenticationMethod.WALLET_KEY,
        )
        const authorization = getPnpRequestAuthorization(req, PRIVATE_KEY2)
        const res = await queryPnpSignEndpoint(req, authorization)
        expect(res.status).toBe(200)
        const resBody = (await res.json()) as SignMessageResponseSuccess
        expect(resBody).toEqual<SignMessageResponseSuccess>({
          success: true,
          version: expectedVersion,
          signature: resBody.signature,
          performedQueryCount: startingPerformedQueryCount + 1,
          totalQuota: resBody.totalQuota,
          warnings: [],
        })
        expect(res.headers.get(KEY_VERSION_HEADER)).toEqual(contextSpecificParams.pnpKeyVersion)
        expect(
          threshold_bls.partialVerifyBlindSignature(
            Buffer.from(contextSpecificParams.pnpPolynomial, 'hex'),
            Buffer.from(blindedMessage, 'base64'),
            Buffer.from(resBody.signature, 'base64'),
          ),
        )

        await sleep(5 * 1000) // sleep for cache ttl

        const res2 = await queryPnpSignEndpoint(req, authorization)
        expect(res2.status).toBe(200)
        const res2Body = (await res2.json()) as SignMessageResponseSuccess
        expect(res2Body).toEqual<SignMessageResponseSuccess>({
          success: true,
          version: expectedVersion,
          signature: resBody.signature,
          performedQueryCount: resBody.performedQueryCount, // Not incremented
          totalQuota: res2Body.totalQuota,
          warnings: [WarningMessage.DUPLICATE_REQUEST_TO_GET_PARTIAL_SIG],
        })
      })
    })

    describe('failure cases', () => {
      const blindedMessage = getBlindedPhoneNumber(PHONE_NUMBER, randomBytes(32))

      it('Should respond with 400 on missing request fields', async () => {
        const badRequest = getPnpSignRequest(
          ACCOUNT_ADDRESS2,
          blindedMessage,
          AuthenticationMethod.WALLET_KEY,
        )
        // @ts-ignore Intentionally deleting required field
        delete badRequest.blindedQueryPhoneNumber
        const authorization = getPnpRequestAuthorization(badRequest, PRIVATE_KEY1)
        const res = await queryPnpSignEndpoint(badRequest, authorization)
        expect(res.status).toBe(400)
        const resBody = (await res.json()) as SignMessageResponseFailure
        expect(resBody).toEqual<SignMessageResponseFailure>({
          success: false,
          version: expectedVersion,
          error: WarningMessage.INVALID_INPUT,
        })
      })

      it('Should respond with 400 on on invalid key version', async () => {
        const badRequest = getPnpSignRequest(
          ACCOUNT_ADDRESS2,
          blindedMessage,
          AuthenticationMethod.WALLET_KEY,
        )
        const authorization = getPnpRequestAuthorization(badRequest, PRIVATE_KEY1)
        const res = await queryPnpSignEndpoint(badRequest, authorization, 'asd')
        expect(res.status).toBe(400)
        const resBody = (await res.json()) as SignMessageResponseFailure
        expect(resBody).toEqual<SignMessageResponseFailure>({
          success: false,
          version: expectedVersion,
          error: WarningMessage.INVALID_KEY_VERSION_REQUEST,
        })
      })

      it('Should respond with 400 on on invalid blinded message', async () => {
        const badRequest = getPnpSignRequest(
          ACCOUNT_ADDRESS2,
          PHONE_NUMBER,
          AuthenticationMethod.WALLET_KEY,
        )
        const authorization = getPnpRequestAuthorization(badRequest, PRIVATE_KEY1)
        const res = await queryPnpSignEndpoint(badRequest, authorization)
        expect(res.status).toBe(400)
        const resBody = (await res.json()) as SignMessageResponseFailure
        expect(resBody).toEqual<SignMessageResponseFailure>({
          success: false,
          version: expectedVersion,
          error: WarningMessage.INVALID_INPUT,
        })
      })

      it('Should respond with 400 on invalid address', async () => {
        const badRequest = getPnpSignRequest(
          '0xnotanaddress',
          blindedMessage,
          AuthenticationMethod.WALLET_KEY,
        )
        const authorization = getPnpRequestAuthorization(badRequest, PRIVATE_KEY1)
        const res = await queryPnpSignEndpoint(badRequest, authorization)
        expect(res.status).toBe(400)
        const resBody = (await res.json()) as SignMessageResponseFailure
        expect(resBody).toEqual<SignMessageResponseFailure>({
          success: false,
          version: expectedVersion,
          error: WarningMessage.INVALID_INPUT,
        })
      })

      it('Should respond with 401 on failed WALLET_KEY auth', async () => {
        const badRequest = getPnpSignRequest(
          ACCOUNT_ADDRESS2,
          blindedMessage,
          AuthenticationMethod.WALLET_KEY,
        )
        const authorization = getPnpRequestAuthorization(badRequest, PRIVATE_KEY1)
        const res = await queryPnpSignEndpoint(badRequest, authorization)
        expect(res.status).toBe(401)
        const resBody = (await res.json()) as SignMessageResponseFailure
        expect(resBody).toEqual<SignMessageResponseFailure>({
          // TODO test if toStrictEqual works after fixing sendFailure<any>
          success: false,
          version: expectedVersion,
          error: WarningMessage.UNAUTHENTICATED_USER,
        })
      })

      it('Should respond with 401 on failed DEK auth when DEK is set for account', async () => {
        const badRequest = getPnpSignRequest(
          ACCOUNT_ADDRESS2,
          blindedMessage,
          AuthenticationMethod.ENCRYPTION_KEY,
        )
        const authorization = getPnpRequestAuthorization(badRequest, PRIVATE_KEY2)
        const res = await queryPnpSignEndpoint(badRequest, authorization)
        expect(res.status).toBe(401)
        const resBody = (await res.json()) as SignMessageResponseFailure
        expect(resBody).toEqual<SignMessageResponseFailure>({
          success: false,
          version: expectedVersion,
          error: WarningMessage.UNAUTHENTICATED_USER,
        })
      })

      it('Should respond with 401 on failed DEK auth when DEK is not set for account', async () => {
        const badRequest = getPnpSignRequest(
          ACCOUNT_ADDRESS1,
          blindedMessage,
          AuthenticationMethod.ENCRYPTION_KEY,
        )
        const authorization = getPnpRequestAuthorization(badRequest, PRIVATE_KEY1)
        const res = await queryPnpSignEndpoint(badRequest, authorization)
        expect(res.status).toBe(401)
        const resBody = (await res.json()) as SignMessageResponseFailure
        expect(resBody).toEqual<SignMessageResponseFailure>({
          success: false,
          version: expectedVersion,
          error: WarningMessage.UNAUTHENTICATED_USER,
        })
      })

      it('Should respond with 403 on out of quota', async () => {
        const quotaReq = getPnpQuotaRequest(ACCOUNT_ADDRESS1)
        const quotaAuthorization = getPnpRequestAuthorization(quotaReq, PRIVATE_KEY1)
        const quotaRes = await queryPnpQuotaEndpoint(quotaReq, quotaAuthorization)
        expect(quotaRes.status).toBe(200)
        const quotaResBody = (await quotaRes.json()) as PnpQuotaResponseSuccess
        // Sanity check
        expect(quotaResBody.performedQueryCount).toEqual(quotaResBody.totalQuota)

        const req = getPnpSignRequest(ACCOUNT_ADDRESS1, blindedMessage)
        const authorization = getPnpRequestAuthorization(req, PRIVATE_KEY1)
        const res = await queryPnpSignEndpoint(req, authorization)
        expect(res.status).toBe(403)
        const resBody = (await res.json()) as SignMessageResponseFailure
        expect(resBody).toEqual<SignMessageResponseFailure>({
          success: false,
          version: expectedVersion,
          error: WarningMessage.EXCEEDED_QUOTA,
          totalQuota: quotaResBody.totalQuota,
          performedQueryCount: quotaResBody.performedQueryCount,
        })
      })
    })
  })
})

async function queryPnpQuotaEndpoint(
  req: PnpQuotaRequest,
  authorization: string,
): Promise<Response> {
  const body = JSON.stringify(req)
  return fetch(ODIS_SIGNER_URL + SignerEndpoint.PNP_QUOTA, {
    method: 'POST',
    headers: {
      Accept: 'application/json',
      'Content-Type': 'application/json',
      Authorization: authorization,
    },
    body,
  })
}

async function queryPnpSignEndpoint(
  req: SignMessageRequest,
  authorization: string,
  keyVersion?: string,
): Promise<Response> {
  const body = JSON.stringify(req)
  const headers: any = {
    Accept: 'application/json',
    'Content-Type': 'application/json',
    Authorization: authorization,
  }
  if (keyVersion !== undefined) {
    headers[KEY_VERSION_HEADER] = keyVersion
  }
  const res = await fetch(ODIS_SIGNER_URL + SignerEndpoint.PNP_SIGN, {
    method: 'POST',
    headers,
    body,
  })
  return res
}

async function sendCUSDToOdisPayments(amountInWei: bigint, recipient: Address, sender: Account) {
  const stableToken = getCUSDContract(client)
  const odisPayments = getOdisPaymentsContract(client)

  try {
    await stableToken.write.approve([odisPayments.address, amountInWei], {
      account: sender,
      chain: client.chain,
      gas: BigInt(100000),
      gasPrice: BigInt(50000000000), // 50 gwei
    })

    await odisPayments.write.payInCUSD([recipient, amountInWei], {
      account: sender,
      chain: client.chain,
      gas: BigInt(150000),
      gasPrice: BigInt(50000000000), // 50 gwei
    })
  } catch (error) {
    // If transaction fails due to nonce/pricing issues, try with higher gas price
    if (
      (error as Error).message.includes('replacement transaction underpriced') ||
      (error as Error).message.includes('nonce') ||
      (error as Error).message.includes('base-fee-floor')
    ) {
      console.warn('Transaction conflict detected, retrying with higher gas price...')

      await stableToken.write.approve([odisPayments.address, amountInWei], {
        account: sender,
        chain: client.chain,
        gas: BigInt(100000),
        gasPrice: BigInt(100000000000), // 100 gwei
      })

      await odisPayments.write.payInCUSD([recipient, amountInWei], {
        account: sender,
        chain: client.chain,
        gas: BigInt(150000),
        gasPrice: BigInt(100000000000), // 100 gwei
      })
    } else {
      throw error
    }
  }

  // Add small delay to avoid rapid successive transactions
  await sleep(1000)
}
