import * as t from 'io-ts'
import { OdisRequest, PhoneNumberPrivacyRequest, PnpQuotaRequest, SignMessageRequest } from '.'

// Phone Number Privacy
export interface PnpQuotaStatus {
  performedQueryCount: number
  // all time total quota
  totalQuota: number
}

const PnpQuotaStatusSchema: t.Type<PnpQuotaStatus> = t.type({
  performedQueryCount: t.number,
  totalQuota: t.number,
})

export interface SignMessageResponseSuccess extends PnpQuotaStatus {
  success: true
  version: string
  signature: string
  warnings?: string[]
}

export interface SignMessageResponseFailure {
  success: false
  version: string
  error: string
  // These fields are occasionally provided by the signer but not the combiner
  // because the combiner separates failure/success responses before processing states.
  // => If the signer response fails, then it's irrelevant if that signer returned quota values,
  // since these won't be used in the quota calculation anyways.
  // Changing this is more involved; TODO(future) https://github.com/celo-org/celo-monorepo/issues/9826
  performedQueryCount?: number
  totalQuota?: number
}

export type SignMessageResponse = SignMessageResponseSuccess | SignMessageResponseFailure

export const SignMessageResponseSchema: t.Type<SignMessageResponse> = t.union([
  t.intersection([
    t.type({
      success: t.literal(true),
      version: t.string,
      signature: t.string,
    }),
    t.partial({
      warnings: t.union([t.array(t.string), t.undefined]),
    }),
    PnpQuotaStatusSchema,
  ]),
  t.intersection([
    t.type({
      success: t.literal(false),
      version: t.string,
      error: t.string,
    }),
    t.partial({
      performedQueryCount: t.union([t.number, t.undefined]),
      totalQuota: t.union([t.number, t.undefined]),
    }),
  ]),
])

export interface PnpQuotaResponseSuccess extends PnpQuotaStatus {
  success: true
  version: string
  warnings?: string[]
}

export interface PnpQuotaResponseFailure {
  success: false
  version: string
  error: string
}

export type PnpQuotaResponse = PnpQuotaResponseSuccess | PnpQuotaResponseFailure

export const PnpQuotaResponseSchema: t.Type<PnpQuotaResponse> = t.union([
  t.intersection([
    t.type({
      success: t.literal(true),
      version: t.string,
    }),
    t.partial({
      warnings: t.union([t.array(t.string), t.undefined]),
    }),
    PnpQuotaStatusSchema,
  ]),
  t.type({
    success: t.literal(false),
    version: t.string,
    error: t.string,
  }),
])

// prettier-ignore
export type PhoneNumberPrivacyResponse<
  R extends PhoneNumberPrivacyRequest = PhoneNumberPrivacyRequest
> =
  | R extends SignMessageRequest ? SignMessageResponse : never
  | R extends PnpQuotaRequest ? PnpQuotaResponse : never

// General

// prettier-ignore
export type OdisResponse<R extends OdisRequest = OdisRequest> =
  | R extends PhoneNumberPrivacyRequest ? PhoneNumberPrivacyResponse<R> : never

export type SuccessResponse<R extends OdisRequest = OdisRequest> = OdisResponse<R> & {
  success: true
}
export type FailureResponse<R extends OdisRequest = OdisRequest> = OdisResponse<R> & {
  success: false
}
