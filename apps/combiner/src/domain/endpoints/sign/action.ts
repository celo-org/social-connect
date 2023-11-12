import {
  CombinerEndpoint,
  DomainRestrictedSignatureRequest,
  domainRestrictedSignatureRequestSchema,
  domainRestrictedSignatureResponseSchema,
  DomainSchema,
  ErrorMessage,
  ErrorType,
  getSignerEndpoint,
  OdisResponse,
  SequentialDelayDomainStateSchema,
  verifyDomainRestrictedSignatureRequestAuthenticity,
  WarningMessage,
} from '@celo/phone-number-privacy-common'
import assert from 'node:assert'
import { Signer, thresholdCallToSigners } from '../../../common/combine'
import { Context } from '../../../common/context'
import { DomainCryptoClient } from '../../../common/crypto-clients/domain-crypto-client'
import { errorResult, ResultHandler } from '../../../common/handlers'
import { getKeyVersionInfo, requestHasSupportedKeyVersion } from '../../../common/io'
import { Counters } from '../../../common/metrics'
import { getCombinerVersion, OdisConfig } from '../../../config'
import { logDomainResponseDiscrepancies } from '../../services/log-responses'
import { findThresholdDomainState } from '../../services/threshold-state'

export function domainSign(
  signers: Signer[],
  config: OdisConfig,
): ResultHandler<DomainRestrictedSignatureRequest> {
  return async (request, response) => {
    const { logger } = response.locals
    const { url } = request
    const ctx: Context = { url, logger }

    if (!domainRestrictedSignatureRequestSchema(DomainSchema).is(request.body)) {
      Counters.warnings.labels(request.url, WarningMessage.INVALID_INPUT).inc()
      return errorResult(400, WarningMessage.INVALID_INPUT)
    }
    if (!requestHasSupportedKeyVersion(request, config, logger)) {
      Counters.warnings.labels(request.url, WarningMessage.INVALID_KEY_VERSION_REQUEST).inc()
      return errorResult(400, WarningMessage.INVALID_KEY_VERSION_REQUEST)
    }

    // Note that signing requests may include a nonce for replay protection that will be checked by
    // the signer, but is not checked here. As a result, requests that pass the authentication check
    // here may still fail when sent to the signer.
    if (!verifyDomainRestrictedSignatureRequestAuthenticity(request.body)) {
      Counters.warnings.labels(request.url, WarningMessage.UNAUTHENTICATED_USER).inc()
      return errorResult(401, WarningMessage.UNAUTHENTICATED_USER)
    }

    const keyVersionInfo = getKeyVersionInfo(request, config, logger)
    const crypto = new DomainCryptoClient(keyVersionInfo)

    const processResult = async (
      res: OdisResponse<DomainRestrictedSignatureRequest>,
    ): Promise<boolean> => {
      assert(res.success)
      // TODO remove the need to pass url here
      crypto.addSignature({ url: request.url, signature: res.signature }, ctx)

      // Send response immediately once we cross threshold
      // BLS threshold signatures can be combined without all partial signatures
      if (crypto.hasSufficientSignatures()) {
        try {
          crypto.combineBlindedSignatureShares(request.body.blindedMessage, ctx)
          // Close outstanding requests
          return true
        } catch (err) {
          // One or more signatures failed verification and were discarded.
          logger.info('Error caught in receiveSuccess')
          logger.info(err)
          // Continue to collect signatures.
        }
      }
      return false
    }

    const { signerResponses, maxErrorCode } = await thresholdCallToSigners(
      ctx,
      {
        signers,
        endpoint: getSignerEndpoint(CombinerEndpoint.DOMAIN_SIGN),
        request,
        keyVersionInfo,
        requestTimeoutMS: config.odisServices.timeoutMilliSeconds,
        responseSchema: domainRestrictedSignatureResponseSchema(SequentialDelayDomainStateSchema),
        shouldCheckKeyVersion: true,
      },
      processResult,
    )

    logDomainResponseDiscrepancies(response.locals.logger, signerResponses)

    if (crypto.hasSufficientSignatures()) {
      try {
        const combinedSignature = crypto.combineBlindedSignatureShares(
          request.body.blindedMessage,
          ctx,
        )

        return {
          status: 200,
          body: {
            success: true,
            version: getCombinerVersion(),
            signature: combinedSignature,
            status: findThresholdDomainState(keyVersionInfo, signerResponses, signers.length),
          },
        }
      } catch (err) {
        // May fail upon combining signatures if too many sigs are invalid
        logger.error('Combining signatures failed in combine')
        logger.error(err)
        // Fallback to handleMissingSignatures
      }
    }

    const errorCode = maxErrorCode ?? 500
    const error = errorCodeToError(errorCode)
    if (error == ErrorMessage.NOT_ENOUGH_PARTIAL_SIGNATURES) {
      Counters.errors.labels(request.url, error).inc()
      Counters.notEnoughSigErrors.labels(request.url).inc()
    } else if (error in WarningMessage) {
      Counters.warnings.labels(request.url, error).inc()
    }
    return errorResult(errorCode, error)
  }
}

function errorCodeToError(errorCode: number): ErrorType {
  switch (errorCode) {
    case 429:
      return WarningMessage.EXCEEDED_QUOTA
    case 401:
      // Authentication is checked in the combiner, but invalid nonces are passed through
      return WarningMessage.INVALID_NONCE
    default:
      return ErrorMessage.NOT_ENOUGH_PARTIAL_SIGNATURES
  }
}
