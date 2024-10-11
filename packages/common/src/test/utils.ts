import { privateKeyToAddress } from '@celo/utils/lib/address'
import { serializeSignature, signMessage } from '@celo/utils/lib/signatureUtils'
import BigNumber from 'bignumber.js'
import {
  AuthenticationMethod,
  PhoneNumberPrivacyRequest,
  PnpQuotaRequest,
  SignMessageRequest,
} from '../interfaces'
import { signWithRawKey } from '../utils/authentication'
import { genSessionID } from '../utils/logger'

export interface AttestationsStatus {
  isVerified: boolean
  numAttestationsRemaining: number
  total: number
  completed: number
}

export function createMockAttestation(getVerifiedStatus: jest.Mock<AttestationsStatus, []>) {
  return {
    getVerifiedStatus,
  }
}

export function createMockAccounts(
  getWalletAddress: jest.Mock<string, []>,
  getDataEncryptionKey: jest.Mock<string, []>,
) {
  return {
    read: {
      getWalletAddress,
      getDataEncryptionKey,
    },
  }
}

// Take in jest.Mock to enable individual tests to spy on function calls
// and more easily set return values
export function createMockOdisPayments(totalPaidCUSDFunc: jest.Mock<BigNumber, []>) {
  return {
    read: { totalPaidCUSD: totalPaidCUSDFunc },
  }
}

export enum ContractRetrieval {
  getStableToken = 'getStableToken',
  getGoldToken = 'getGoldToken',
  getAccounts = 'getAccounts',
  getOdisPayments = 'getOdisPayments',
}

export function getPnpQuotaRequest(
  account: string,
  authenticationMethod?: string,
): PnpQuotaRequest {
  return {
    account,
    authenticationMethod,
    sessionID: genSessionID(),
  }
}

export function getPnpSignRequest(
  account: string,
  blindedQueryPhoneNumber: string,
  authenticationMethod?: string,
): SignMessageRequest {
  return {
    account,
    blindedQueryPhoneNumber,
    authenticationMethod,
    sessionID: genSessionID(),
  }
}

export function getPnpRequestAuthorization(req: PhoneNumberPrivacyRequest, pk: string) {
  const msg = JSON.stringify(req)
  if (req.authenticationMethod === AuthenticationMethod.ENCRYPTION_KEY) {
    return signWithRawKey(JSON.stringify(req), pk)
  }
  const account = privateKeyToAddress(pk)
  return serializeSignature(signMessage(msg, pk, account))
}
