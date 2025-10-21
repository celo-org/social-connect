import { hexToBuffer } from '@celo/base'
import Logger from 'bunyan'
import { Request } from 'express'
import { Client, createClient, http } from 'viem'
import { celoSepolia } from 'viem/chains'
import { getAccountsContract } from '../../src/contracts'
import { ErrorMessage, ErrorType } from '../../src/interfaces/errors'
import { AuthenticationMethod } from '../../src/interfaces/requests'
import * as auth from '../../src/utils/authentication'
import { newDEKFetcher } from '../../src/utils/authentication'

// Mock the getAccountsContract function
jest.mock('../../src/contracts', () => {
  const originalModule = jest.requireActual('../../src/contracts')
  return {
    ...originalModule,
    getAccountsContract: jest.fn(),
  }
})

const mockGetAccountsContract = getAccountsContract as jest.MockedFunction<
  typeof getAccountsContract
>

describe('Authentication test suite', () => {
  const logger = Logger.createLogger({
    name: 'logger',
    level: 'warn',
  })

  const client = createClient({ transport: http(), chain: celoSepolia })

  // Helper function to create mock accounts contract
  const createMockAccountsContract = (dekReturnValue: string | Promise<string> | Error) => ({
    read: {
      getDataEncryptionKey: jest.fn().mockImplementation(() => {
        if (dekReturnValue instanceof Error) {
          throw dekReturnValue
        }
        return Promise.resolve(dekReturnValue)
      }),
    },
  })

  beforeEach(() => {
    jest.clearAllMocks()
  })

  describe('authenticateUser utility', () => {
    it("Should fail authentication with missing 'Authorization' header", async () => {
      const sampleRequest: Request = {
        get: (_: string) => '',
        body: {
          account: '0xc1912fee45d61c87cc5ea59dae31190fffff232d',
        },
      } as Request
      const dekFetcher = newDEKFetcher({} as Client, logger)
      const warnings: ErrorType[] = []

      const success = await auth.authenticateUser(sampleRequest, logger, dekFetcher, warnings)

      expect(success).toBe(false)
      expect(warnings).toEqual([])
    })

    it('Should fail authentication with missing signer', async () => {
      const sampleRequest: Request = {
        get: (name: string) => (name === 'Authorization' ? 'Test' : ''),
        body: {},
      } as Request
      const dekFetcher = newDEKFetcher({} as Client, logger)

      const warnings: ErrorType[] = []

      const success = await auth.authenticateUser(sampleRequest, logger, dekFetcher, warnings)

      expect(success).toBe(false)
      expect(warnings).toEqual([])
    })

    it('Should fail authentication with error in getDataEncryptionKey', async () => {
      const sampleRequest: Request = {
        get: (name: string) => (name === 'Authorization' ? 'Test' : ''),
        body: {
          account: '0xc1912fee45d61c87cc5ea59dae31190fffff232d',
          authenticationMethod: AuthenticationMethod.ENCRYPTION_KEY,
        },
      } as Request

      // Mock the contract to throw an error
      mockGetAccountsContract.mockReturnValue(
        createMockAccountsContract(new Error('Connection error')) as any,
      )

      const dekFetcher = newDEKFetcher(client, logger)
      const warnings: ErrorType[] = []

      const success = await auth.authenticateUser(sampleRequest, logger, dekFetcher, warnings)

      expect(success).toBe(false)
      expect(warnings).toEqual([ErrorMessage.FAILURE_TO_GET_DEK])
    })

    it('Should fail authentication when key is not registered', async () => {
      const sampleRequest: Request = {
        get: (name: string) => (name === 'Authorization' ? 'Test' : ''),
        body: {
          account: '0xc1912fee45d61c87cc5ea59dae31190fffff232d',
          authenticationMethod: AuthenticationMethod.ENCRYPTION_KEY,
        },
      } as Request

      // Mock the contract to return empty string (no registered key)
      mockGetAccountsContract.mockReturnValue(createMockAccountsContract('0x') as any)

      const dekFetcher = newDEKFetcher(client, logger)
      const warnings: ErrorType[] = []

      const success = await auth.authenticateUser(sampleRequest, logger, dekFetcher, warnings)

      expect(success).toBe(false)
      expect(warnings).toEqual([])
    })

    it('Should fail authentication when key is registered but not valid', async () => {
      const sampleRequest: Request = {
        get: (name: string) => (name === 'Authorization' ? 'Test' : ''),
        body: {
          account: '0xc1912fee45d61c87cc5ea59dae31190fffff232d',
          authenticationMethod: AuthenticationMethod.ENCRYPTION_KEY,
        },
      } as Request

      // Mock the contract to return an invalid key
      mockGetAccountsContract.mockReturnValue(
        createMockAccountsContract('notAValidKeyEncryption') as any,
      )

      const dekFetcher = newDEKFetcher(client, logger)
      const warnings: ErrorType[] = []

      const success = await auth.authenticateUser(sampleRequest, logger, dekFetcher, warnings)

      expect(success).toBe(false)
      expect(warnings).toEqual([])
    })

    it('Should succeed authentication when key is registered and valid', async () => {
      const rawKey = '41e8e8593108eeedcbded883b8af34d2f028710355c57f4c10a056b72486aa04'
      const body = {
        account: '0xc1912fee45d61c87cc5ea59dae31190fffff232d',
        authenticationMethod: AuthenticationMethod.ENCRYPTION_KEY,
      }
      const sig = auth.signWithRawKey(JSON.stringify(body), rawKey)
      const sampleRequest: Request = {
        get: (name: string) => (name === 'Authorization' ? sig : ''),
        body,
      } as Request

      // Mock the contract to return the correct public key
      // NOTE: elliptic is disabled elsewhere in this library to prevent
      // accidental signing of truncated messages.
      const EC = require('elliptic').ec
      const ec = new EC('secp256k1')
      const key = ec.keyFromPrivate(hexToBuffer(rawKey))
      const publicKey = key.getPublic(true, 'hex')

      mockGetAccountsContract.mockReturnValue(createMockAccountsContract(publicKey) as any)

      const warnings: ErrorType[] = []
      const dekFetcher = newDEKFetcher(client, logger)

      const success = await auth.authenticateUser(sampleRequest, logger, dekFetcher, warnings)

      expect(success).toBe(true)
      expect(warnings).toEqual([])
    })

    it('Should fail authentication when the message is manipulated', async () => {
      const rawKey = '41e8e8593108eeedcbded883b8af34d2f028710355c57f4c10a056b72486aa04'
      const body = {
        account: '0xc1912fee45d61c87cc5ea59dae31190fffff232d',
        authenticationMethod: AuthenticationMethod.ENCRYPTION_KEY,
      }
      const message = JSON.stringify(body)

      // Mock the contract to return the correct public key
      const EC = require('elliptic').ec
      const ec = new EC('secp256k1')
      const key = ec.keyFromPrivate(hexToBuffer(rawKey))
      const publicKey = key.getPublic(true, 'hex')

      mockGetAccountsContract.mockReturnValue(createMockAccountsContract(publicKey) as any)

      // Modify every fourth character and check that the signature becomes invalid.
      for (let i = 0; i < message.length; i += 4) {
        const modified =
          message.slice(0, i) +
          String.fromCharCode(message.charCodeAt(i) + 1) +
          message.slice(i + 1)
        const sig = auth.signWithRawKey(modified, rawKey)
        const sampleRequest: Request = {
          get: (name: string) => (name === 'Authorization' ? sig : ''),
          body,
        } as Request

        const warnings: ErrorType[] = []
        const dekFetcher = newDEKFetcher(client, logger)

        const success = await auth.authenticateUser(sampleRequest, logger, dekFetcher, warnings)

        expect(success).toBe(false)
        expect(warnings).toEqual([])
      }
    })

    it('Should fail authentication when the key is incorrect', async () => {
      const rawKey = '41e8e8593108eeedcbded883b8af34d2f028710355c57f4c10a056b72486aa04'
      const body = {
        account: '0xc1912fee45d61c87cc5ea59dae31190fffff232d',
        authenticationMethod: AuthenticationMethod.ENCRYPTION_KEY,
      }
      const sig = auth.signWithRawKey(JSON.stringify(body), rawKey)
      const sampleRequest: Request = {
        get: (name: string) => (name === 'Authorization' ? sig : ''),
        body,
      } as Request

      // Mock the contract to return a manipulated key
      const EC = require('elliptic').ec
      const ec = new EC('secp256k1')
      // Send back a manipulated key.
      const manipulatedKey = ec.keyFromPrivate(hexToBuffer('a' + rawKey.slice(1)))
      const wrongPublicKey = manipulatedKey.getPublic(true, 'hex')

      mockGetAccountsContract.mockReturnValue(createMockAccountsContract(wrongPublicKey) as any)

      const warnings: ErrorType[] = []
      const dekFetcher = newDEKFetcher(client, logger)

      const success = await auth.authenticateUser(sampleRequest, logger, dekFetcher, warnings)

      expect(success).toBe(false)
      expect(warnings).toEqual([])
    })

    it('Should fail authentication when the sigature is modified', async () => {
      const rawKey = '41e8e8593108eeedcbded883b8af34d2f028710355c57f4c10a056b72486aa04'
      const body = {
        account: '0xc1912fee45d61c87cc5ea59dae31190fffff232d',
        authenticationMethod: AuthenticationMethod.ENCRYPTION_KEY,
      }
      // Manipulate the signature.
      const sig = auth.signWithRawKey(JSON.stringify(body), rawKey)
      const modified = JSON.stringify([0] + JSON.parse(sig))
      const sampleRequest: Request = {
        get: (name: string) => (name === 'Authorization' ? modified : ''),
        body,
      } as Request

      // Mock the contract to return the correct public key
      const EC = require('elliptic').ec
      const ec = new EC('secp256k1')
      const key = ec.keyFromPrivate(hexToBuffer(rawKey))
      const publicKey = key.getPublic(true, 'hex')

      mockGetAccountsContract.mockReturnValue(createMockAccountsContract(publicKey) as any)

      const warnings: ErrorType[] = []
      const dekFetcher = newDEKFetcher(client, logger)

      const success = await auth.authenticateUser(sampleRequest, logger, dekFetcher, warnings)

      expect(success).toBe(false)
      expect(warnings).toEqual([])
    })

    it('Should fail authentication when key is registered and valid and signature is incorrectly generated', async () => {
      const rawKey = '41e8e8593108eeedcbded883b8af34d2f028710355c57f4c10a056b72486aa04'
      const body = {
        account: '0xc1912fee45d61c87cc5ea59dae31190fffff232d',
        authenticationMethod: AuthenticationMethod.ENCRYPTION_KEY,
      }
      // NOTE: elliptic is disabled elsewhere in this library to prevent
      // accidental signing of truncated messages.
      const EC = require('elliptic').ec
      const ec = new EC('secp256k1')
      const key = ec.keyFromPrivate(hexToBuffer(rawKey))
      const sig = JSON.stringify(key.sign(JSON.stringify(body)).toDER())

      const sampleRequest: Request = {
        get: (name: string) => (name === 'Authorization' ? sig : ''),
        body,
      } as Request

      // Mock the contract to return the correct public key
      const publicKey = key.getPublic(true, 'hex')
      mockGetAccountsContract.mockReturnValue(createMockAccountsContract(publicKey) as any)

      const dekFetcher = newDEKFetcher(client, logger)
      const warnings: ErrorType[] = []

      const success = await auth.authenticateUser(sampleRequest, logger, dekFetcher, warnings)

      expect(success).toBe(false)
      expect(warnings).toEqual([])
    })
  })
})
