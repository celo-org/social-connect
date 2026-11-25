import * as t from 'io-ts'
import { KEY_VERSION_HEADER } from '..'

export enum AuthenticationMethod {
  WALLET_KEY = 'wallet_key',
  ENCRYPTION_KEY = 'encryption_key',
}

export interface SignMessageRequest {
  /** Celo account address. Query is charged against this account's quota. */
  account: string
  /** Query message. A blinded elliptic curve point encoded in base64. */
  blindedQueryPhoneNumber: string
  /** Authentication method to use for verifying the signature in the Authorization header */
  authenticationMethod?: string
  /** Client-specified session ID for the request. */
  sessionID?: string
  /** Client-specified version string */
  version?: string
}

export const SignMessageRequestSchema: t.Type<SignMessageRequest> = t.intersection([
  t.type({
    account: t.string,
    blindedQueryPhoneNumber: t.string,
  }),
  t.partial({
    authenticationMethod: t.union([t.string, t.undefined]),
    sessionID: t.union([t.string, t.undefined]),
    version: t.union([t.string, t.undefined]),
  }),
])

export interface PnpQuotaRequest {
  account: string
  /** Authentication method to use for verifying the signature in the Authorization header */
  authenticationMethod?: string
  /** Client-specified session ID for the request. */
  sessionID?: string
  /** Client-specified version string */
  version?: string
}

export const PnpQuotaRequestSchema: t.Type<PnpQuotaRequest> = t.intersection([
  t.type({
    account: t.string,
  }),
  t.partial({
    authenticationMethod: t.union([t.string, t.undefined]),
    sessionID: t.union([t.string, t.undefined]),
    version: t.union([t.string, t.undefined]),
  }),
])

export type PhoneNumberPrivacyRequest = SignMessageRequest | PnpQuotaRequest

export type OdisRequest = PhoneNumberPrivacyRequest

interface PnpAuthHeader {
  Authorization: string
}

interface KeyVersionHeader {
  [KEY_VERSION_HEADER]?: string
}

export type SignMessageRequestHeader = KeyVersionHeader & PnpAuthHeader

export type PnpQuotaRequestHeader = PnpAuthHeader

export type PhoneNumberPrivacyRequestHeader<R extends PhoneNumberPrivacyRequest> =
  R extends SignMessageRequest
    ? SignMessageRequestHeader
    : never | R extends PnpQuotaRequest
      ? PnpQuotaRequestHeader
      : never

export type OdisRequestHeader<R extends OdisRequest> = R extends PhoneNumberPrivacyRequest
  ? PhoneNumberPrivacyRequestHeader<R>
  : never
