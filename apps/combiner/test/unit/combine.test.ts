import {
  DomainQuotaStatusRequest,
  domainQuotaStatusResponseSchema,
  KeyVersionInfo,
  SequentialDelayDomainStateSchema,
  SignerEndpoint,
} from '@celo/phone-number-privacy-common'
import Logger from 'bunyan'
import fetch, { Response } from 'node-fetch'
import { thresholdCallToSigners } from '../../src/common/combine'
import { findThresholdDomainState } from '../../src/domain/services/threshold-state'

jest.mock('node-fetch', () => {
  const actual = jest.requireActual('node-fetch')
  return { __esModule: true, ...actual, default: jest.fn() }
})

describe('thresholdCallToSigners', () => {
  it('keeps ODIS error bodies but ignores non-ODIS ones (e.g. a proxy error page)', async () => {
    const now = 1000
    const json = (body: object, status: number) => new Response(JSON.stringify(body), { status })
    const bySigner: Record<string, () => Response> = {
      // Reverse proxy error page with a top-level `status`, returned before any real signer answers.
      'http://proxy': () => json({ title: 'Error 1016: Origin DNS error', status: 530 }, 530),
      'http://odis-error': () => json({ success: false, version: '1', error: 'boom' }, 500),
      ...Object.fromEntries(
        [1, 2, 2, 3].map((counter, i) => [
          `http://s${i}`,
          () =>
            json(
              {
                success: true,
                version: '1',
                status: { counter, timer: now - 1, disabled: false, now },
              },
              200,
            ),
        ]),
      ),
    }
    ;(fetch as unknown as jest.Mock).mockImplementation((url: string) =>
      Promise.resolve(bySigner[url.replace(SignerEndpoint.DOMAIN_QUOTA_STATUS, '')]()),
    )

    const keyVersionInfo: KeyVersionInfo = {
      keyVersion: 1,
      threshold: 3,
      polynomial: 'mock',
      pubKey: 'mock',
    }
    const signers = Object.keys(bySigner).map((url) => ({ url }))
    const request = { url: '/domain/quotaStatus', headers: {}, body: {} } as any
    const { signerResponses } = await thresholdCallToSigners<DomainQuotaStatusRequest>(
      { url: request.url, logger: Logger.createLogger({ name: 'test', level: 'fatal' }) },
      {
        signers,
        endpoint: SignerEndpoint.DOMAIN_QUOTA_STATUS,
        request,
        keyVersionInfo,
        requestTimeoutMS: 1000,
        responseSchema: domainQuotaStatusResponseSchema(SequentialDelayDomainStateSchema),
        shouldCheckKeyVersion: false,
      },
    )

    expect(signerResponses.map((r) => r.url).sort()).toEqual(
      ['http://odis-error', 'http://s0', 'http://s1', 'http://s2', 'http://s3'].sort(),
    )
    expect(findThresholdDomainState(keyVersionInfo, signerResponses, signers.length)).toEqual({
      counter: 2,
      timer: now - 1,
      disabled: false,
      now,
    })
  })
})
