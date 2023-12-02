import {
  ErrorMessage,
  KeyVersionInfo,
  OdisRequest,
  OdisResponse,
  responseHasExpectedKeyVersion,
  SignerEndpoint,
  WarningMessage,
} from '@celo/phone-number-privacy-common'
import Logger from 'bunyan'
import { Request } from 'express'
import * as t from 'io-ts'
import { Context } from './context'
import { fetchSignerResponseWithFallback, SignerResponse } from './io'
import { Counters, Histograms, newMeter } from './metrics'

export interface Signer {
  url: string
  fallbackUrl?: string
}

export interface ThresholdCallToSignersOptions<R extends OdisRequest> {
  signers: Signer[]
  endpoint: SignerEndpoint
  requestTimeoutMS: number
  shouldCheckKeyVersion: boolean
  keyVersionInfo: KeyVersionInfo
  request: Request<{}, {}, R>
  responseSchema: t.Type<OdisResponse<R>, OdisResponse<R>, unknown>
}

export async function thresholdCallToSigners<R extends OdisRequest>(
  ctx: Context,
  options: ThresholdCallToSignersOptions<R>,
  processResult: (res: OdisResponse<R>) => Promise<boolean> = (_) => Promise.resolve(false),
): Promise<{ signerResponses: Array<SignerResponse<R>>; maxErrorCode?: number }> {
  const {
    signers,
    endpoint,
    requestTimeoutMS,
    shouldCheckKeyVersion,
    keyVersionInfo,
    request,
    responseSchema,
  } = options

  const { logger } = ctx

  const manualAbort = new AbortController()
  // @ts-ignore
  const timeoutSignal = AbortSignal.timeout(requestTimeoutMS)
  const abortSignal = abortSignalAny([manualAbort.signal, timeoutSignal])

  let errorCount = 0
  const errorCodes: Map<number, number> = new Map<number, number>()

  const requiredThreshold = keyVersionInfo.threshold

  const responses: Array<SignerResponse<R>> = []
  // Forward request to signers
  // An unexpected error in handling the result for one signer should not
  // block a threshold of correct responses, but should be logged.
  await Promise.all(
    signers.map(async (signer) => {
      try {
        const _meter = newMeter(Histograms.signerLatency, request.url, signer.url)
        const signerFetchResult = await _meter(() =>
          fetchSignerResponseWithFallback(
            signer,
            endpoint,
            keyVersionInfo.keyVersion,
            request,
            logger,
            // @ts-ignore
            abortSignal,
          ),
        )

        Counters.sigResponses
          .labels(signerFetchResult.status.toString(), signer.url, request.url)
          .inc()
        logger.info({
          message: 'Received signerFetchResult',
          signer: signer.url,
          status: signerFetchResult.status,
        })

        if (!signerFetchResult.ok) {
          const responseData = await signerFetchResult.json()
          Counters.sigResponsesErrors
            .labels(signerFetchResult.status.toString(), signer.url, request.url)
            .inc()
          logger.info({
            message: 'Received signerFetchResult on unsuccessful signer response',
            res: responseData,
            status: signerFetchResult.status,
            signer: signer.url,
          })

          errorCount++
          errorCodes.set(
            signerFetchResult.status,
            (errorCodes.get(signerFetchResult.status) ?? 0) + 1,
          )

          if (signers.length - errorCount < requiredThreshold) {
            logger.warn('Not possible to reach a threshold of signer responses. Failing fast')
            manualAbort.abort()
          }
          responses.push({ res: responseData, url: signer.url })
          return
        }

        if (
          shouldCheckKeyVersion &&
          !responseHasExpectedKeyVersion(signerFetchResult, keyVersionInfo.keyVersion, logger)
        ) {
          throw new Error(ErrorMessage.INVALID_KEY_VERSION_RESPONSE)
        }

        const data: any = await signerFetchResult.json()
        logger.info(
          { signer, res: data, status: signerFetchResult.status },
          `received 'OK' response from signer`,
        )

        const odisResponse: OdisResponse<R> = parseSchema(responseSchema, data, logger)
        if (!odisResponse.success) {
          logger.error(
            { err: odisResponse.error, signer: signer.url },
            `Signer request to failed with 'OK' status`,
          )
          throw new Error(ErrorMessage.SIGNER_RESPONSE_FAILED_WITH_OK_STATUS)
        }

        responses.push({ res: odisResponse, url: signer.url })

        if (await processResult(odisResponse)) {
          // we already have enough responses
          manualAbort.abort()
        }
      } catch (err) {
        if (isTimeoutError(err)) {
          Counters.sigRequestErrors
            .labels(signer.url, request.url, ErrorMessage.TIMEOUT_FROM_SIGNER)
            .inc()
          logger.error({ signer }, ErrorMessage.TIMEOUT_FROM_SIGNER)
        } else if (isAbortError(err)) {
          Counters.warnings.labels(request.url, WarningMessage.CANCELLED_REQUEST_TO_SIGNER).inc()
          logger.info({ signer }, WarningMessage.CANCELLED_REQUEST_TO_SIGNER)
        } else {
          Counters.sigRequestErrors
            .labels(signer.url, request.url, ErrorMessage.SIGNER_REQUEST_ERROR)
            .inc()
          logger.error({ signer, err }, ErrorMessage.SIGNER_REQUEST_ERROR)

          errorCount++
          if (signers.length - errorCount < requiredThreshold) {
            logger.warn('Not possible to reach a threshold of signer responses. Failing fast')
            manualAbort.abort()
          }
        }
      }
    }),
  )

  if (errorCodes.size > 0) {
    if (errorCodes.size > 1) {
      Counters.sigInconsistenciesErrors.labels(request.url).inc()
      logger.error(
        { errorCodes: JSON.stringify([...errorCodes]) },
        ErrorMessage.INCONSISTENT_SIGNER_RESPONSES,
      )
    }

    return { signerResponses: responses, maxErrorCode: getMajorityErrorCode(errorCodes) }
  } else {
    return { signerResponses: responses }
  }
}

function parseSchema<T>(schema: t.Type<T, T, unknown>, data: unknown, logger: Logger): T {
  if (!schema.is(data)) {
    logger.error({ data }, `Malformed schema`)
    throw new Error(ErrorMessage.INVALID_SIGNER_RESPONSE)
  }
  return data
}

function isTimeoutError(err: unknown) {
  return err instanceof Error && err.name === 'TimeoutError'
}

export function isAbortError(err: unknown) {
  return err instanceof Error && err.name === 'AbortError'
}

function getMajorityErrorCode(errorCodes: Map<number, number>): number {
  let maxErrorCode = -1
  let maxCount = -1
  errorCodes.forEach((count, errorCode) => {
    // This gives priority to the lower status codes in the event of a tie
    // because 400s are more helpful than 500s for user feedback
    if (count > maxCount || (count === maxCount && errorCode < maxErrorCode)) {
      maxCount = count
      maxErrorCode = errorCode
    }
  })
  return maxErrorCode
}

/*
 * TODO remove this in favor of actual implementation once we can upgrade to node v18.17.0.
 * The Combiner cannot currently be deployed with node versions beyond v18.
 * Actual implementation:
 * https://github.com/nodejs/node/blob/5ff1ead6b2d6da7ba044b11e2824c7cbf5a94cb8/lib/internal/abort_controller.js#L198C24-L198C24
 */
function abortSignalAny(signals: AbortSignal[]): AbortSignal {
  const ac = new AbortController()
  for (const signal of signals) {
    if (signal.aborted) {
      ac.abort(signal)
      return ac.signal
    }
    signal.addEventListener(
      'abort',
      (e) => {
        ac.abort(e)
      },
      { once: true },
    )
  }
  return ac.signal
}
