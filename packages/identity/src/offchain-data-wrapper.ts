import { ensureLeading0x } from '@celo/base/lib/address'
import { Err, Ok, Result, RootError, makeAsyncThrowable } from '@celo/base/lib/result'
import { ClaimTypes } from '@celo/contractkit/lib/identity/claims/types'
import { IdentityMetadataWrapper } from '@celo/contractkit/lib/identity/metadata'
import { publicKeyToAddress } from '@celo/utils/lib/address'
import { ensureUncompressed } from '@celo/utils/lib/ecdh'
import {
  recoverEIP712TypedDataSignerRsv,
  recoverEIP712TypedDataSignerVrs,
  verifyEIP712TypedDataSigner,
} from '@celo/utils/lib/signatureUtils'
import fetch from 'cross-fetch'
import debugFactory from 'debug'
import * as t from 'io-ts'
import { WalletClient, type Address } from 'viem'
import { AuthorizedSignerAccessor } from './offchain/accessors/authorized-signer'
import { StorageWriter } from './offchain/storage-writers'
import { buildEIP712TypedData, resolvePath } from './offchain/utils'

import { getAccountsContract } from '@celo/phone-number-privacy-common'

const debug = debugFactory('offchaindata')

export enum OffchainErrorTypes {
  FetchError = 'FetchError',
  InvalidSignature = 'InvalidSignature',
  NoStorageRootProvidedData = 'NoStorageRootProvidedData',
  NoStorageProvider = 'NoStorageProvider',
}

export class FetchError extends RootError<OffchainErrorTypes.FetchError> {
  constructor(error: Error) {
    super(OffchainErrorTypes.FetchError)
    this.message = error.message
  }
}

export class InvalidSignature extends RootError<OffchainErrorTypes.InvalidSignature> {
  constructor() {
    super(OffchainErrorTypes.InvalidSignature)
  }
}

export class NoStorageRootProvidedData extends RootError<OffchainErrorTypes.NoStorageRootProvidedData> {
  constructor() {
    super(OffchainErrorTypes.NoStorageRootProvidedData)
  }
}

export class NoStorageProvider extends RootError<OffchainErrorTypes.NoStorageProvider> {
  constructor() {
    super(OffchainErrorTypes.NoStorageProvider)
  }
}

export type OffchainErrors =
  | FetchError
  | InvalidSignature
  | NoStorageRootProvidedData
  | NoStorageProvider

export interface OffchainDataWrapper {
  viemClient: WalletClient
  signer: Address
  self: Address
  writeDataTo(data: Buffer, signature: Buffer, dataPath: string): Promise<OffchainErrors | void>
  readDataFromAsResult<DataType>(
    account: Address,
    dataPath: string,
    checkOffchainSigners: boolean,
    type?: t.Type<DataType>,
  ): Promise<Result<Buffer, OffchainErrors>>
}

export class BasicDataWrapper implements OffchainDataWrapper {
  storageWriter: StorageWriter | undefined
  signer: Address

  constructor(
    readonly self: Address,
    readonly viemClient: WalletClient,
    signer?: Address,
  ) {
    this.signer = signer || self
  }

  async readDataFromAsResult<DataType>(
    account: Address,
    dataPath: string,
    checkOffchainSigners: boolean,
    type?: t.Type<DataType>,
  ): Promise<Result<Buffer, OffchainErrors>> {
    const accounts = getAccountsContract(this.viemClient)

    const metadataURL = await accounts.read.getMetadataURL([account])
    debug({ account, metadataURL })

    const accountsWrapper = {
      isAccount: async (address: string) => accounts.read.isAccount([address as Address]),
      getValidatorSigner: async (address: string) =>
        accounts.read.getValidatorSigner([address as Address]),
      getVoteSigner: async (address: string) =>
        accounts.read.getValidatorSigner([address as Address]),
      getAttestationSigner: async (address: string) =>
        accounts.read.getValidatorSigner([address as Address]),
    }

    const minimalKit = {
      contracts: {
        getAccounts: async () => accountsWrapper,
      },
    }

    // @ts-expect-error it might not look it but the above are the only methods called on the kit. yes this is very dangerous
    const metadata = await IdentityMetadataWrapper.fetchFromURL(minimalKit, metadataURL)
    // TODO: Filter StorageRoots with the datapath glob
    const storageRoots = metadata
      .filterClaims(ClaimTypes.STORAGE)
      .map((_) => new StorageRoot(this, account, _.address))

    if (storageRoots.length === 0) {
      return Err(new NoStorageRootProvidedData())
    }

    const results = await Promise.all(
      storageRoots.map(async (s) => s.readAndVerifySignature(dataPath, checkOffchainSigners, type)),
    )
    const item = results.find((s) => s.ok)

    if (item === undefined) {
      return Err(new NoStorageRootProvidedData())
    }

    return item
  }

  readDataFrom = makeAsyncThrowable(this.readDataFromAsResult.bind(this))

  async writeDataTo(
    data: Buffer,
    signature: Buffer,
    dataPath: string,
  ): Promise<OffchainErrors | void> {
    if (this.storageWriter === undefined) {
      return new NoStorageProvider()
    }

    try {
      await Promise.all([
        this.storageWriter.write(data, dataPath),
        this.storageWriter.write(signature, `${dataPath}.signature`),
      ])
    } catch (e: any) {
      return new FetchError(e instanceof Error ? e : new Error(e))
    }
  }
}

class StorageRoot {
  constructor(
    readonly wrapper: OffchainDataWrapper,
    readonly account: Address,
    readonly root: string,
  ) {}

  async readAndVerifySignature<DataType>(
    dataPath: string,
    checkOffchainSigners: boolean,
    type?: t.Type<DataType>,
  ): Promise<Result<Buffer, OffchainErrors>> {
    let dataResponse, signatureResponse

    try {
      ;[dataResponse, signatureResponse] = await Promise.all([
        fetch(resolvePath(this.root, dataPath)),
        fetch(resolvePath(this.root, `${dataPath}.signature`)),
      ])
    } catch (error: any) {
      const fetchError = error instanceof Error ? error : new Error(error)
      return Err(new FetchError(fetchError))
    }

    if (!dataResponse.ok) {
      return Err(new FetchError(new Error(dataResponse.statusText)))
    }
    if (!signatureResponse.ok) {
      return Err(new FetchError(new Error(signatureResponse.statusText)))
    }

    const [dataBody, signatureBody] = await Promise.all([
      dataResponse.arrayBuffer(),
      signatureResponse.arrayBuffer(),
    ])
    const body = Buffer.from(dataBody)
    const signature = ensureLeading0x(Buffer.from(signatureBody).toString('hex'))

    const toParse = type ? JSON.parse(body.toString()) : body
    const typedData = await buildEIP712TypedData(this.wrapper, dataPath, toParse, type)

    if (verifyEIP712TypedDataSigner(typedData, signature, this.account)) {
      return Ok(body)
    }

    const accountsContract = getAccountsContract(this.wrapper.viemClient)
    if (await accountsContract.read.isAccount([this.account])) {
      const keys = await Promise.all([
        accountsContract.read.getVoteSigner([this.account]),
        accountsContract.read.getValidatorSigner([this.account]),
        accountsContract.read.getAttestationSigner([this.account]),
        accountsContract.read.getDataEncryptionKey([this.account]),
      ])

      const dekAddress = keys[3] ? publicKeyToAddress(ensureUncompressed(keys[3])) : '0x0'
      const signers = [keys[0], keys[1], keys[2], dekAddress]

      if (signers.some((signer) => verifyEIP712TypedDataSigner(typedData, signature, signer))) {
        return Ok(body)
      }

      if (checkOffchainSigners) {
        let guessedSigner: string
        try {
          guessedSigner = recoverEIP712TypedDataSignerRsv(typedData, signature)
        } catch (error) {
          guessedSigner = recoverEIP712TypedDataSignerVrs(typedData, signature)
        }
        const authorizedSignerAccessor = new AuthorizedSignerAccessor(this.wrapper)
        const authorizedSigner = await authorizedSignerAccessor.readAsResult(
          this.account,
          guessedSigner,
        )
        if (authorizedSigner.ok) {
          return Ok(body)
        }
      }
    }

    return Err(new InvalidSignature())
  }
}
