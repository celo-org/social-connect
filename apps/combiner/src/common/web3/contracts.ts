import { ErrorMessage, getDataEncryptionKey } from '@celo/phone-number-privacy-common'
import Logger from 'bunyan'
import { Address, Client } from 'viem'
import config from '../../config'
import { Counters, Histograms, newMeter } from '../metrics'

export async function getDEK(client: Client, logger: Logger, account: Address): Promise<string> {
  const _meter = newMeter(Histograms.fullNodeLatency, 'getDataEncryptionKey')
  return _meter(() =>
    getDataEncryptionKey(
      account,
      client,
      logger,
      config.phoneNumberPrivacy.fullNodeTimeoutMs,
      config.phoneNumberPrivacy.fullNodeRetryCount,
      config.phoneNumberPrivacy.fullNodeRetryDelayMs,
    ).catch((err) => {
      logger.error({ err, account }, 'failed to get on-chain DEK for account')
      Counters.errors.labels('NA', ErrorMessage.FULL_NODE_ERROR).inc()
      Counters.blockchainErrors.labels('NA', ErrorMessage.FAILURE_TO_GET_DEK).inc()
      throw err
    }),
  )
}
