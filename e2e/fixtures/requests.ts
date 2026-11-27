import {
  AuthenticationMethod,
  OdisRequest,
  PnpQuotaRequest,
  SignMessageRequest,
  signWithRawKey,
} from '@celo/identity'
import { privateKeyToAddress } from '@celo/utils/lib/address'
import { serializeSignature, signMessage } from '@celo/utils/lib/signatureUtils'

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

export function getPnpRequestAuthorization(req: OdisRequest, pk: string) {
  const msg = JSON.stringify(req)
  if (req.authenticationMethod === AuthenticationMethod.ENCRYPTION_KEY) {
    return signWithRawKey(JSON.stringify(req), pk)
  }
  const account = privateKeyToAddress(pk)
  return serializeSignature(signMessage(msg, pk, account))
}

function genSessionID() {
  return Math.random().toString(36).substring(2, 15) + Math.random().toString(36).substring(2, 15)
}
