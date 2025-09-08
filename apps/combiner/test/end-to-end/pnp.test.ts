import { sleep } from '@celo/base'
import { OdisUtils } from '@celo/identity'
import { ErrorMessages, getServiceContext, OdisAPI } from '@celo/identity/lib/odis/query'
import { PnpClientQuotaStatus } from '@celo/identity/lib/odis/quota'
import {
  CombinerEndpoint,
  getAccountsContract,
  getCUSDContract,
  getOdisPaymentsContract,
  PnpQuotaRequest,
  PnpQuotaResponseSchema,
  SignMessageRequest,
  SignMessageResponseSchema,
} from '@celo/phone-number-privacy-common'
import { config as signerConfig } from '@celo/phone-number-privacy-signer/src/config'
import { normalizeAddressWith0x } from '@celo/utils/lib/address'
import threshold_bls from 'blind-threshold-bls'
import { randomBytes } from 'crypto'
import fetch from 'node-fetch'
import { Hex } from 'viem'
import { getCombinerVersion } from '../../src'
import {
  ACCOUNT_ADDRESS,
  ACCOUNT_ADDRESS_NO_QUOTA,
  BLINDED_PHONE_NUMBER,
  client,
  dekAuthSigner,
  deks,
  getTestContextName,
  PHONE_NUMBER,
  walletAuthSigner,
} from './resources'

const { IdentifierPrefix } = OdisUtils.Identifier

require('dotenv').config()

jest.setTimeout(60000)

const SERVICE_CONTEXT = getServiceContext(getTestContextName(), OdisAPI.PNP)
const combinerUrl = SERVICE_CONTEXT.odisUrl
const fullNodeUrl = process.env.ODIS_BLOCKCHAIN_PROVIDER

const expectedVersion = getCombinerVersion()

describe(`Running against service deployed at ${combinerUrl} w/ blockchain provider ${fullNodeUrl}`, () => {
  beforeAll(async () => {
    const accountsContract = getAccountsContract(client)
    const dekPublicKey = normalizeAddressWith0x(deks[0].publicKey) as Hex
    if ((await accountsContract.read.getDataEncryptionKey([ACCOUNT_ADDRESS])) !== dekPublicKey) {
      await accountsContract.write.setAccountDataEncryptionKey([dekPublicKey], {
        account: client.account,
        chain: client.chain,
      })
    }
  })

  it('Service is deployed at correct version', async () => {
    const response = await fetch(combinerUrl + CombinerEndpoint.STATUS, {
      method: 'GET',
    })
    const body = await response.json()
    // This checks against local package.json version, change if necessary
    expect(body.version).toBe(expectedVersion)
  })

  describe(`${CombinerEndpoint.PNP_QUOTA}`, () => {
    it('Should succeed when authenticated with WALLET_KEY', async () => {
      const res = await OdisUtils.Quota.getPnpQuotaStatus(
        ACCOUNT_ADDRESS,
        walletAuthSigner,
        SERVICE_CONTEXT,
      )
      expect(res).toStrictEqual<PnpClientQuotaStatus>({
        version: expectedVersion,
        performedQueryCount: res.performedQueryCount,
        totalQuota: res.totalQuota,
        remainingQuota: res.totalQuota - res.performedQueryCount,
        warnings: res.warnings ?? [],
      })
    })

    it('Should succeed when authenticated with DEK', async () => {
      const res = await OdisUtils.Quota.getPnpQuotaStatus(
        ACCOUNT_ADDRESS,
        dekAuthSigner(0),
        SERVICE_CONTEXT,
      )
      expect(res).toStrictEqual<PnpClientQuotaStatus>({
        version: expectedVersion,
        performedQueryCount: res.performedQueryCount,
        totalQuota: res.totalQuota,
        remainingQuota: res.totalQuota - res.performedQueryCount,
        warnings: res.warnings ?? [],
      })
    })

    it('Should succeed on repeated valid requests', async () => {
      const res1 = await OdisUtils.Quota.getPnpQuotaStatus(
        ACCOUNT_ADDRESS,
        dekAuthSigner(0),
        SERVICE_CONTEXT,
      )
      const expectedRes: PnpClientQuotaStatus = {
        version: expectedVersion,
        performedQueryCount: res1.performedQueryCount,
        totalQuota: res1.totalQuota,
        remainingQuota: res1.totalQuota - res1.performedQueryCount,
        warnings: res1.warnings ?? [],
      }
      expect(res1).toStrictEqual<PnpClientQuotaStatus>(expectedRes)
      const res2 = await OdisUtils.Quota.getPnpQuotaStatus(
        ACCOUNT_ADDRESS,
        dekAuthSigner(0),
        SERVICE_CONTEXT,
      )
      expect(res2).toStrictEqual<PnpClientQuotaStatus>(expectedRes)
    })

    it(`Should reject to throw ${ErrorMessages.ODIS_INPUT_ERROR} with invalid address`, async () => {
      await expect(
        OdisUtils.Quota.getPnpQuotaStatus('not an address', dekAuthSigner(0), SERVICE_CONTEXT),
      ).rejects.toThrow(ErrorMessages.ODIS_INPUT_ERROR)
    })

    it(`Should reject to throw ${ErrorMessages.ODIS_AUTH_ERROR} with invalid WALLET_KEY auth`, async () => {
      const req: PnpQuotaRequest = {
        account: ACCOUNT_ADDRESS,
        authenticationMethod: walletAuthSigner.authenticationMethod,
      }
      await expect(
        OdisUtils.Query.queryOdis(
          req,
          SERVICE_CONTEXT,
          CombinerEndpoint.PNP_QUOTA,
          PnpQuotaResponseSchema,
          {
            Authorization: await walletAuthSigner.sign191({
              message: JSON.stringify(req),
              account: ACCOUNT_ADDRESS_NO_QUOTA,
            }),
          },
        ),
      ).rejects.toThrow(ErrorMessages.ODIS_AUTH_ERROR)
    })

    it(`Should reject to throw ${ErrorMessages.ODIS_AUTH_ERROR} with invalid DEK auth`, async () => {
      await expect(
        OdisUtils.Quota.getPnpQuotaStatus(
          ACCOUNT_ADDRESS,
          dekAuthSigner(1), // DEK auth signer doesn't match the registered DEK for ACCOUNT_ADDRESS
          SERVICE_CONTEXT,
        ),
      ).rejects.toThrow(ErrorMessages.ODIS_AUTH_ERROR)
    })
  })

  describe(`${CombinerEndpoint.PNP_SIGN}`, () => {
    beforeAll(async () => {
      // Check current quota and only replenish if needed
      const currentQuota = await OdisUtils.Quota.getPnpQuotaStatus(
        ACCOUNT_ADDRESS,
        dekAuthSigner(0),
        SERVICE_CONTEXT,
      )

      if (currentQuota.remainingQuota < 50) {
        console.log(`Current remaining quota: ${currentQuota.remainingQuota}, replenishing...`)
        // Replenish quota for ACCOUNT_ADDRESS
        // If this fails, may be necessary to faucet ACCOUNT_ADDRESS more funds
        const numQueriesToReplenish = 100
        const amountInWei = BigInt(
          signerConfig.quota.queryPriceInCUSD.times(1e18).times(numQueriesToReplenish).toString(),
        )
        const stableToken = getCUSDContract(client)
        const odisPayments = getOdisPaymentsContract(client)

        try {
          // Check current allowance
          const currentAllowance = await stableToken.read.allowance([
            client.account.address,
            odisPayments.address,
          ])

          // Only approve if current allowance is insufficient
          if (currentAllowance < amountInWei) {
            await stableToken.write.approve([odisPayments.address, amountInWei], {
              account: client.account,
              chain: client.chain,
              gas: BigInt(100000),
              gasPrice: BigInt(100000000000), // 100 gwei - increased to avoid replacement transaction underpriced
            })
          }

          await odisPayments.write.payInCUSD([ACCOUNT_ADDRESS, amountInWei], {
            account: client.account,
            chain: client.chain,
            gas: BigInt(150000),
            gasPrice: BigInt(100000000000), // 100 gwei - increased to avoid replacement transaction underpriced
          })
          // wait for cache to expire and then query to refresh
          await sleep(5 * 1000)
          await OdisUtils.Quota.getPnpQuotaStatus(
            ACCOUNT_ADDRESS,
            dekAuthSigner(0),
            SERVICE_CONTEXT,
          )
        } catch (error) {
          console.warn(
            'Failed to replenish quota, continuing with existing quota:',
            (error as Error).message,
          )
        }
      } else {
        console.log(
          `Current remaining quota: ${currentQuota.remainingQuota}, no replenishment needed`,
        )
      }
    })

    describe('new requests', () => {
      // Requests made for PHONE_NUMBER from ACCOUNT_ADDRESS & same blinding factor
      // are replayed from previous test runs (for every run after the very first)
      let startingPerformedQueryCount: number
      let startingTotalQuota: number
      beforeEach(async () => {
        const res = await OdisUtils.Quota.getPnpQuotaStatus(
          ACCOUNT_ADDRESS,
          dekAuthSigner(0),
          SERVICE_CONTEXT,
        )
        startingPerformedQueryCount = res.performedQueryCount
        startingTotalQuota = res.totalQuota
      })

      it('Should increment performedQueryCount on success with DEK auth', async () => {
        // Raw key is used as the blinding client's seed, so we need a new PN
        // Create a fake PN that is always incrementing and shouldn't ever repeat
        const unusedPN = `+1${Date.now()}`
        await OdisUtils.Identifier.getObfuscatedIdentifier(
          unusedPN,
          IdentifierPrefix.PHONE_NUMBER,
          ACCOUNT_ADDRESS,
          dekAuthSigner(0),
          SERVICE_CONTEXT,
        )
        const quotaRes = await OdisUtils.Quota.getPnpQuotaStatus(
          ACCOUNT_ADDRESS,
          dekAuthSigner(0),
          SERVICE_CONTEXT,
        )
        expect(quotaRes.version).toBe(expectedVersion)
        expect(quotaRes.performedQueryCount).toBe(startingPerformedQueryCount + 1)
        expect(quotaRes.totalQuota).toBe(startingTotalQuota)
        expect(quotaRes.remainingQuota).toBe(startingTotalQuota - (startingPerformedQueryCount + 1))
        expect(Array.isArray(quotaRes.warnings)).toBe(true)
      })

      it('Should increment performedQueryCount on success with WALLET_KEY auth', async () => {
        await OdisUtils.Identifier.getObfuscatedIdentifier(
          PHONE_NUMBER,
          IdentifierPrefix.PHONE_NUMBER,
          ACCOUNT_ADDRESS,
          walletAuthSigner,
          SERVICE_CONTEXT,
          Buffer.from(randomBytes(32)).toString('base64'),
        )
        const quotaRes = await OdisUtils.Quota.getPnpQuotaStatus(
          ACCOUNT_ADDRESS,
          walletAuthSigner,
          SERVICE_CONTEXT,
        )
        expect(quotaRes.version).toBe(expectedVersion)
        expect(quotaRes.performedQueryCount).toBe(startingPerformedQueryCount + 1)
        expect(quotaRes.totalQuota).toBe(startingTotalQuota)
        expect(quotaRes.remainingQuota).toBe(startingTotalQuota - (startingPerformedQueryCount + 1))
        expect(Array.isArray(quotaRes.warnings)).toBe(true)
      })
    })

    describe('replayed requests', () => {
      const replayedBlindingFactor = Buffer.from('test string for blinding factor').toString(
        'base64',
      )
      beforeAll(async () => {
        // Ensure that these are each called at least once for the first test runs
        await OdisUtils.Identifier.getObfuscatedIdentifier(
          PHONE_NUMBER,
          IdentifierPrefix.PHONE_NUMBER,
          ACCOUNT_ADDRESS,
          walletAuthSigner,
          SERVICE_CONTEXT,
          replayedBlindingFactor,
        )
        await OdisUtils.Identifier.getObfuscatedIdentifier(
          PHONE_NUMBER,
          IdentifierPrefix.PHONE_NUMBER,
          ACCOUNT_ADDRESS,
          dekAuthSigner(0),
          SERVICE_CONTEXT,
        )
      })

      // Requests made for PHONE_NUMBER from ACCOUNT_ADDRESS
      // are replayed from previous test runs (for every run after the very first)
      let startingPerformedQueryCount: number
      let startingTotalQuota: number
      beforeEach(async () => {
        const res = await OdisUtils.Quota.getPnpQuotaStatus(
          ACCOUNT_ADDRESS,
          dekAuthSigner(0),
          SERVICE_CONTEXT,
        )
        startingPerformedQueryCount = res.performedQueryCount
        startingTotalQuota = res.totalQuota
      })

      it('Should succeed and not update performedQueryCount when authenticated with WALLET_KEY', async () => {
        const res = await OdisUtils.Identifier.getObfuscatedIdentifier(
          PHONE_NUMBER,
          IdentifierPrefix.PHONE_NUMBER,
          ACCOUNT_ADDRESS,
          walletAuthSigner,
          SERVICE_CONTEXT,
          replayedBlindingFactor,
        )
        threshold_bls.verify(
          Buffer.from(SERVICE_CONTEXT.odisPubKey, 'base64'),
          Buffer.from(PHONE_NUMBER),
          Buffer.from(res.unblindedSignature!, 'base64'),
        )
        const quotaRes = await OdisUtils.Quota.getPnpQuotaStatus(
          ACCOUNT_ADDRESS,
          walletAuthSigner,
          SERVICE_CONTEXT,
        )
        expect(quotaRes.performedQueryCount).toEqual(startingPerformedQueryCount)
        expect(quotaRes.totalQuota).toEqual(startingTotalQuota)
      })

      it('Should succeed and not update performedQueryCount when authenticated with DEK', async () => {
        const res = await OdisUtils.Identifier.getObfuscatedIdentifier(
          PHONE_NUMBER,
          IdentifierPrefix.PHONE_NUMBER,
          ACCOUNT_ADDRESS,
          dekAuthSigner(0),
          SERVICE_CONTEXT,
        )
        threshold_bls.verify(
          Buffer.from(SERVICE_CONTEXT.odisPubKey, 'base64'),
          Buffer.from(PHONE_NUMBER),
          Buffer.from(res.unblindedSignature!, 'base64'),
        )
        const quotaRes = await OdisUtils.Quota.getPnpQuotaStatus(
          ACCOUNT_ADDRESS,
          dekAuthSigner(0),
          SERVICE_CONTEXT,
        )
        expect(quotaRes.performedQueryCount).toEqual(startingPerformedQueryCount)
        expect(quotaRes.totalQuota).toEqual(startingTotalQuota)
      })
    })

    // NOTE: these are also replayed requests
    for (let i = 1; i <= 2; i++) {
      it(`Should succeed on valid request with key version header ${i}`, async () => {
        const res = await OdisUtils.Identifier.getObfuscatedIdentifier(
          PHONE_NUMBER,
          IdentifierPrefix.PHONE_NUMBER,
          ACCOUNT_ADDRESS,
          dekAuthSigner(0),
          SERVICE_CONTEXT,
          undefined,
          undefined,
          undefined,
          undefined,
          i,
        )
        threshold_bls.verify(
          Buffer.from(SERVICE_CONTEXT.odisPubKey, 'base64'),
          Buffer.from(PHONE_NUMBER),
          Buffer.from(res.unblindedSignature!, 'base64'),
        )
      })
    }

    it(`Should succeed on invalid key version`, async () => {
      const res = await OdisUtils.Identifier.getObfuscatedIdentifier(
        PHONE_NUMBER,
        IdentifierPrefix.PHONE_NUMBER,
        ACCOUNT_ADDRESS,
        dekAuthSigner(0),
        SERVICE_CONTEXT,
        undefined,
        undefined,
        undefined,
        undefined,
        1.5,
      )
      threshold_bls.verify(
        Buffer.from(SERVICE_CONTEXT.odisPubKey, 'base64'),
        Buffer.from(PHONE_NUMBER),
        Buffer.from(res.unblindedSignature!, 'base64'),
      )
    })

    it(`Should reject to throw ${ErrorMessages.ODIS_INPUT_ERROR} on unsupported key version`, async () => {
      await expect(
        OdisUtils.Identifier.getObfuscatedIdentifier(
          PHONE_NUMBER,
          IdentifierPrefix.PHONE_NUMBER,
          ACCOUNT_ADDRESS,
          dekAuthSigner(0),
          SERVICE_CONTEXT,
          undefined,
          undefined,
          undefined,
          undefined,
          10,
        ),
      ).rejects.toThrow(ErrorMessages.ODIS_INPUT_ERROR)
    })

    it(`Should reject to throw ${ErrorMessages.ODIS_INPUT_ERROR} on invalid address`, async () => {
      await expect(
        OdisUtils.Identifier.getObfuscatedIdentifier(
          PHONE_NUMBER,
          IdentifierPrefix.PHONE_NUMBER,
          'not an address',
          dekAuthSigner(0),
          SERVICE_CONTEXT,
          undefined,
          undefined,
          undefined,
          undefined,
          1,
        ),
      ).rejects.toThrow(ErrorMessages.ODIS_INPUT_ERROR)
    })

    it(`Should reject to throw ${ErrorMessages.ODIS_INPUT_ERROR} on invalid phone number`, async () => {
      await expect(
        OdisUtils.Identifier.getObfuscatedIdentifier(
          '12345',
          IdentifierPrefix.PHONE_NUMBER,
          ACCOUNT_ADDRESS,
          dekAuthSigner(0),
          SERVICE_CONTEXT,
          undefined,
          undefined,
          undefined,
          undefined,
          1,
        ),
      ).rejects.toThrow('Invalid phone number: 12345')
    })

    it(`Should reject to throw ${ErrorMessages.ODIS_AUTH_ERROR} with invalid WALLET_KEY auth`, async () => {
      const req: SignMessageRequest = {
        account: ACCOUNT_ADDRESS,
        blindedQueryPhoneNumber: BLINDED_PHONE_NUMBER,
        authenticationMethod: walletAuthSigner.authenticationMethod,
      }
      await expect(
        OdisUtils.Query.queryOdis(
          req,
          SERVICE_CONTEXT,
          CombinerEndpoint.PNP_SIGN,
          SignMessageResponseSchema,
          {
            Authorization: await walletAuthSigner.sign191({
              message: JSON.stringify(req),
              account: ACCOUNT_ADDRESS_NO_QUOTA,
            }),
          },
        ),
      ).rejects.toThrow(ErrorMessages.ODIS_AUTH_ERROR)
    })

    it(`Should reject to throw ${ErrorMessages.ODIS_AUTH_ERROR} with invalid DEK auth`, async () => {
      await expect(
        OdisUtils.Identifier.getObfuscatedIdentifier(
          PHONE_NUMBER,
          IdentifierPrefix.PHONE_NUMBER,
          ACCOUNT_ADDRESS,
          dekAuthSigner(1), // DEK auth signer doesn't match the registered DEK for ACCOUNT_ADDRESS
          SERVICE_CONTEXT,
        ),
      ).rejects.toThrow(ErrorMessages.ODIS_AUTH_ERROR)
    })

    it(`Should reject to throw ${ErrorMessages.ODIS_QUOTA_ERROR} when account has no quota`, async () => {
      await expect(
        OdisUtils.Identifier.getObfuscatedIdentifier(
          PHONE_NUMBER,
          IdentifierPrefix.PHONE_NUMBER,
          ACCOUNT_ADDRESS_NO_QUOTA,
          walletAuthSigner, // Use wallet auth since this account doesn't have the correct DEK registered
          SERVICE_CONTEXT,
        ),
      ).rejects.toThrow(ErrorMessages.ODIS_QUOTA_ERROR)
    })
  })
})
