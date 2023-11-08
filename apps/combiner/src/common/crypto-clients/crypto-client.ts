import { ErrorMessage, KeyVersionInfo } from '@celo/phone-number-privacy-common'
import { Context } from '../context'
import { Histograms } from '../metrics'

export interface ServicePartialSignature {
  url: string
  signature: string
}

export abstract class CryptoClient {
  protected unverifiedSignatures: ServicePartialSignature[] = []
  private tailLatencyTimer: () => void = () => {}

  constructor(protected readonly keyVersionInfo: KeyVersionInfo) {}

  /**
   * Returns true if the number of valid signatures is enough to perform a combination
   */
  public hasSufficientSignatures(): boolean {
    return this.allSignaturesLength >= this.keyVersionInfo.threshold
  }

  public addSignature(serviceResponse: ServicePartialSignature, ctx: Context): void {
    if (!this.allSignaturesLength) {
      // start timer when first signer responds
      this.tailLatencyTimer = Histograms.signerTailLatency.labels(ctx.url).startTimer()
    }
    this.unverifiedSignatures.push(serviceResponse)
  }

  /*
   * Computes the signature for the blinded phone number using subclass-specific
   * logic defined in _combineBlindedSignatureShares.
   * Throws an exception if not enough valid signatures or on aggregation failure.
   */
  public combineBlindedSignatureShares(blindedMessage: string, ctx: Context): string {
    if (!this.hasSufficientSignatures()) {
      const { threshold } = this.keyVersionInfo
      ctx.logger.error(
        { signatures: this.allSignaturesLength, required: threshold },
        ErrorMessage.NOT_ENOUGH_PARTIAL_SIGNATURES,
      )
      throw new Error(
        `${ErrorMessage.NOT_ENOUGH_PARTIAL_SIGNATURES} ${this.allSignaturesLength}/${threshold}`,
      )
    }

    // Once we reach this point, we've received a quorum of signer responses
    this.tailLatencyTimer()

    const timer = Histograms.signatureAggregationLatency.labels(ctx.url).startTimer()
    const combinedSignature = this._combineBlindedSignatureShares(blindedMessage, ctx)
    timer()

    return combinedSignature
  }

  /*
   * Computes the signature for the blinded phone number.
   * Must be implemented by subclass.
   */
  protected abstract _combineBlindedSignatureShares(blindedMessage: string, ctx: Context): string

  /**
   * Returns total number of signatures received; must be implemented by subclass.
   */
  protected abstract get allSignaturesLength(): number
}
