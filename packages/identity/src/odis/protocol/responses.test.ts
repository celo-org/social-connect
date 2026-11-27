import { right } from 'fp-ts/lib/Either'
import { PathReporter } from 'io-ts/lib/PathReporter'
import { PnpQuotaResponseSchema, SignMessageResponseSchema } from './responses'

describe('PnpQuotaResponseSchema', () => {
  it('decodes a success body', () => {
    const body = {
      success: true,
      version: '3.1.2',
      performedQueryCount: 2,
      totalQuota: 10,
      warnings: ['CELO_ODIS_WARN_06 SIGNER Discrepancy found in signers quota measurements'],
    }
    expect(PnpQuotaResponseSchema.decode(body)).toEqual(right(body))
  })

  it('decodes a failure body', () => {
    const body = {
      success: false,
      version: '3.1.2',
      error: 'CELO_ODIS_WARN_02 BAD_INPUT Missing or invalid authentication',
    }
    expect(PnpQuotaResponseSchema.decode(body)).toEqual(right(body))
  })

  it('refuses a success body without totalQuota', () => {
    const body = { success: true, version: '3.1.2', performedQueryCount: 2 }
    expect(PathReporter.report(PnpQuotaResponseSchema.decode(body))).toEqual([
      'Invalid value undefined supplied to : (({ success: true, version: string } & Partial<{ warnings: (Array<string> | undefined) }> & { performedQueryCount: number, totalQuota: number }) | { success: false, version: string, error: string })/0: ({ success: true, version: string } & Partial<{ warnings: (Array<string> | undefined) }> & { performedQueryCount: number, totalQuota: number })/2: { performedQueryCount: number, totalQuota: number }/totalQuota: number',
    ])
  })
})

describe('SignMessageResponseSchema', () => {
  it('decodes a success body', () => {
    const body = {
      success: true,
      version: '3.1.2',
      signature: 'MAAAAAAAAADDilSaA/JGqYYvjiIgeDfmsT1lAg5Hw1+FYG/k/tDDXQ==',
      performedQueryCount: 3,
      totalQuota: 10,
    }
    expect(SignMessageResponseSchema.decode(body)).toEqual(right(body))
  })

  it('decodes a failure body', () => {
    const body = {
      success: false,
      version: '3.1.2',
      error: 'CELO_ODIS_WARN_03 QUOTA Requester exceeded service query quota',
      performedQueryCount: 10,
      totalQuota: 10,
    }
    expect(SignMessageResponseSchema.decode(body)).toEqual(right(body))
  })

  it('refuses a success body without signature', () => {
    const body = { success: true, version: '3.1.2', performedQueryCount: 3, totalQuota: 10 }
    expect(PathReporter.report(SignMessageResponseSchema.decode(body))).toEqual([
      'Invalid value undefined supplied to : (({ success: true, version: string, signature: string } & Partial<{ warnings: (Array<string> | undefined) }> & { performedQueryCount: number, totalQuota: number }) | ({ success: false, version: string, error: string } & Partial<{ performedQueryCount: (number | undefined), totalQuota: (number | undefined) }>))/0: ({ success: true, version: string, signature: string } & Partial<{ warnings: (Array<string> | undefined) }> & { performedQueryCount: number, totalQuota: number })/0: { success: true, version: string, signature: string }/signature: string',
    ])
  })
})
