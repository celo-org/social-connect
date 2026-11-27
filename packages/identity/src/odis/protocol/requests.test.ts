import { right } from 'fp-ts/lib/Either'
import { PathReporter } from 'io-ts/lib/PathReporter'
import { AuthenticationMethod, PnpQuotaRequestSchema, SignMessageRequestSchema } from './requests'

describe('SignMessageRequestSchema', () => {
  it('decodes a request', () => {
    const body = {
      account: '0x6037800E91eaa703e38bad40c01410bbdf0FEa7e',
      blindedQueryPhoneNumber: 'HhK3VvEbbWKRBiINDO38KqDBNBx8X42SiQRWXBCRwxbcpTsbaR8JOmbxVdy5Fk+A',
      authenticationMethod: AuthenticationMethod.WALLET_KEY,
      sessionID: 'session-1',
      version: '6.0.0',
    }
    expect(SignMessageRequestSchema.decode(body)).toEqual(right(body))
  })

  it('refuses a request without blindedQueryPhoneNumber', () => {
    const body = { account: '0x6037800E91eaa703e38bad40c01410bbdf0FEa7e' }
    expect(PathReporter.report(SignMessageRequestSchema.decode(body))).toEqual([
      'Invalid value undefined supplied to : ({ account: string, blindedQueryPhoneNumber: string } & Partial<{ authenticationMethod: (string | undefined), sessionID: (string | undefined), version: (string | undefined) }>)/0: { account: string, blindedQueryPhoneNumber: string }/blindedQueryPhoneNumber: string',
    ])
  })
})

describe('PnpQuotaRequestSchema', () => {
  it('decodes a request', () => {
    const body = {
      account: '0x6037800E91eaa703e38bad40c01410bbdf0FEa7e',
      authenticationMethod: AuthenticationMethod.ENCRYPTION_KEY,
      sessionID: 'session-1',
      version: '6.0.0',
    }
    expect(PnpQuotaRequestSchema.decode(body)).toEqual(right(body))
  })

  it('refuses a request without account', () => {
    const body = { authenticationMethod: AuthenticationMethod.WALLET_KEY }
    expect(PathReporter.report(PnpQuotaRequestSchema.decode(body))).toEqual([
      'Invalid value undefined supplied to : ({ account: string } & Partial<{ authenticationMethod: (string | undefined), sessionID: (string | undefined), version: (string | undefined) }>)/0: { account: string }/account: string',
    ])
  })
})
