import { KEY_VERSION_HEADER } from './constants'

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

export interface PnpQuotaRequest {
  account: string
  /** Authentication method to use for verifying the signature in the Authorization header */
  authenticationMethod?: string
  /** Client-specified session ID for the request. */
  sessionID?: string
  /** Client-specified version string */
  version?: string
}

export type OdisRequest = SignMessageRequest | PnpQuotaRequest

interface PnpAuthHeader {
  Authorization: string
}

interface KeyVersionHeader {
  [KEY_VERSION_HEADER]?: string
}

export type SignMessageRequestHeader = KeyVersionHeader & PnpAuthHeader

export type PnpQuotaRequestHeader = PnpAuthHeader

export type OdisRequestHeader<R extends OdisRequest> = R extends SignMessageRequest
  ? SignMessageRequestHeader
  : R extends PnpQuotaRequest
    ? PnpQuotaRequestHeader
    : never
