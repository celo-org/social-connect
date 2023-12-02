import {
  ErrorMessage,
  ErrorType,
  OdisRequest,
  OdisResponse,
  PnpQuotaStatus,
  send,
  SequentialDelayDomainState,
  WarningMessage,
} from '@celo/phone-number-privacy-common'
import opentelemetry, { SpanStatusCode } from '@opentelemetry/api'
import { SemanticAttributes } from '@opentelemetry/semantic-conventions'
import Logger from 'bunyan'
import { Request, Response } from 'express'
import * as client from 'prom-client'
import { getCombinerVersion } from '../config'
import { OdisError } from './error'
import { Counters, Histograms, newMeter } from './metrics'

const tracer = opentelemetry.trace.getTracer('combiner-tracer')

export interface Locals {
  logger: Logger
}

export type PromiseHandler<R extends OdisRequest> = (
  request: Request<{}, {}, R>,
  res: Response<OdisResponse<R>, Locals>,
) => Promise<void>

export function catchErrorHandler<R extends OdisRequest>(
  handler: PromiseHandler<R>,
): PromiseHandler<R> {
  return async (req, res) => {
    try {
      await handler(req, res)
    } catch (err) {
      const logger: Logger = res.locals.logger
      logger.error(err, ErrorMessage.CAUGHT_ERROR_IN_ENDPOINT_HANDLER)
      Counters.errorsCaughtInEndpointHandler.labels(req.url).inc()
      if (!res.headersSent) {
        if (err instanceof OdisError) {
          Counters.errors.labels(req.url, err.code).inc()
          sendFailure(err.code, err.status, res, req.url)
        } else {
          Counters.errors.labels(req.url, ErrorMessage.UNKNOWN_ERROR).inc()
          Counters.unknownErrors.labels(req.url).inc()
          sendFailure(ErrorMessage.UNKNOWN_ERROR, 500, res, req.url)
        }
      } else {
        Counters.errors.labels(req.url, ErrorMessage.ERROR_AFTER_RESPONSE_SENT).inc()
        logger.error(ErrorMessage.ERROR_AFTER_RESPONSE_SENT)
      }
    }
  }
}

export function tracingHandler<R extends OdisRequest>(
  handler: PromiseHandler<R>,
): PromiseHandler<R> {
  return async (req, res) => {
    return tracer.startActiveSpan(
      req.url,
      {
        attributes: {
          [SemanticAttributes.HTTP_ROUTE]: req.path,
          [SemanticAttributes.HTTP_METHOD]: req.method,
          [SemanticAttributes.HTTP_CLIENT_IP]: req.ip,
        },
      },
      async (span) => {
        try {
          await handler(req, res)
          span.setStatus({
            code: SpanStatusCode.OK,
          })
        } catch (err: any) {
          span.setStatus({
            code: SpanStatusCode.ERROR,
            message: err instanceof Error ? err.message : 'Fail',
          })
          throw err
        } finally {
          span.end()
        }
      },
    )
  }
}

export function meteringHandler<R extends OdisRequest>(
  histogram: client.Histogram<string>,
  handler: PromiseHandler<R>,
): PromiseHandler<R> {
  return async (req, res) =>
    newMeter(
      histogram,
      req.url,
    )(async () => {
      const logger: Logger = res.locals.logger
      logger.info({ req: req.body }, 'Request received')
      Counters.requests.labels(req.url).inc()

      const eventLoopLagTimer = Histograms.eventLoopLag.labels(req.url).startTimer()
      setTimeout(() => {
        eventLoopLagTimer()
      })

      await handler(req, res)
      if (res.headersSent) {
        logger.info({ res }, 'Response sent')
        Counters.responses.labels(req.url, res.statusCode.toString()).inc()
      }
    })
}

export function timeoutHandler<R extends OdisRequest>(
  timeoutMs: number,
  handler: PromiseHandler<R>,
): PromiseHandler<R> {
  return async (req, res) => {
    const timeoutSignal = (AbortSignal as any).timeout(timeoutMs)
    timeoutSignal.addEventListener(
      'abort',
      () => {
        if (!res.headersSent) {
          sendFailure(ErrorMessage.TIMEOUT_FROM_SIGNER, 500, res, req.url)
        }
      },
      { once: true },
    )

    await handler(req, res)
  }
}

export async function disabledHandler<R extends OdisRequest>(
  req: Request<{}, {}, R>,
  response: Response<OdisResponse<R>, Locals>,
): Promise<void> {
  Counters.warnings.labels(req.url, WarningMessage.API_UNAVAILABLE).inc()
  sendFailure(WarningMessage.API_UNAVAILABLE, 503, response, req.url)
}

export function sendFailure(
  error: ErrorType,
  status: number,
  response: Response,
  _endpoint: string,
  body?: Record<any, any>, // TODO remove any
) {
  send(
    response,
    {
      success: false,
      version: getCombinerVersion(),
      error,
      ...body,
    },
    status,
    response.locals.logger,
  )
}

export interface Result<R extends OdisRequest> {
  status: number
  body: OdisResponse<R>
}

export type ResultHandler<R extends OdisRequest> = (
  request: Request<{}, {}, R>,
  res: Response<OdisResponse<R>, Locals>,
) => Promise<Result<R>>

export function resultHandler<R extends OdisRequest>(
  resHandler: ResultHandler<R>,
): PromiseHandler<R> {
  return async (req, res) => {
    const result = await resHandler(req, res)
    send(res, result.body, result.status, res.locals.logger)
  }
}

export function errorResult(
  status: number,
  error: string,
  quotaStatus?: PnpQuotaStatus | { status: SequentialDelayDomainState },
): Result<any> {
  // TODO remove any
  return {
    status,
    body: {
      success: false,
      version: getCombinerVersion(),
      error,
      ...quotaStatus,
    },
  }
}
