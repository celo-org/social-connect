import { Address, trimLeading0x } from '@celo/base'
import { Err, makeAsyncThrowable } from '@celo/base/lib/result'
import { isValidAddress, toChecksumAddress } from '@celo/utils/lib/address'
import { either } from 'fp-ts/lib/Either'
import * as t from 'io-ts'
import { OffchainDataWrapper, OffchainErrors } from '../../offchain-data-wrapper'
import { buildEIP712TypedData, deserialize } from '../utils'
import { OffchainError } from './errors'

// The codec of @celo/utils/lib/io, built on this package's io-ts. @celo/utils pins a different
// fp-ts, so its codecs come from a second io-ts instance whose types t.type does not accept.
const AddressType = new t.Type<string, string, unknown>(
  'Address',
  t.string.is,
  (input, context) =>
    either.chain(t.string.validate(input, context), (stringValue) =>
      isValidAddress(stringValue)
        ? t.success(toChecksumAddress(stringValue))
        : t.failure(stringValue, context, 'is not a valid address'),
    ),
  String,
)

const AuthorizedSignerSchema = t.type({
  address: AddressType,
  proofOfPossession: t.string,
  filteredDataPaths: t.string,
})

export class AuthorizedSignerAccessor {
  basePath = '/account/authorizedSigners'
  constructor(readonly wrapper: OffchainDataWrapper) {}

  async readAsResult(account: Address, signer: Address) {
    const dataPath = this.basePath + '/' + toChecksumAddress(signer)
    const rawData = await this.wrapper.readDataFromAsResult(
      account,
      dataPath,
      false,
      AuthorizedSignerSchema,
    )
    if (!rawData.ok) {
      return Err(new OffchainError(rawData.error))
    }

    return deserialize(AuthorizedSignerSchema, rawData.result)
  }

  read = makeAsyncThrowable(this.readAsResult.bind(this))

  async write(
    signer: Address,
    proofOfPossession: string,
    filteredDataPaths: string,
  ): Promise<OffchainErrors | void> {
    const payload = {
      address: toChecksumAddress(signer),
      proofOfPossession,
      filteredDataPaths,
    }
    const dataPath = this.basePath + '/' + toChecksumAddress(signer)
    const typedData = await buildEIP712TypedData(
      this.wrapper,
      dataPath,
      payload,
      AuthorizedSignerSchema,
    )
    const signature = await this.wrapper.kit
      .getWallet()!
      .signTypedData(this.wrapper.self, typedData)
    return this.wrapper.writeDataTo(
      Buffer.from(JSON.stringify(payload)),
      Buffer.from(trimLeading0x(signature), 'hex'),
      dataPath,
    )
  }
}
