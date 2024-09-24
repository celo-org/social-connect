import { retryAsyncWithBackOffAndTimeout } from '@celo/base'
import { getDataEncryptionKey, getOdisPaymentsContract } from '@celo/phone-number-privacy-common'
import { BigNumber } from 'bignumber.js'
import Logger from 'bunyan'
import { Address, Client, Hex } from 'viem'
import { config } from '../../config'
import { Counters, Histograms, newMeter } from '../metrics'

export async function getOnChainOdisPayments(
  client: Client,
  logger: Logger,
  account: Address,
): Promise<BigNumber> {
  const _meter = newMeter(Histograms.fullNodeLatency, 'getOnChainOdisPayments')
  return _meter(() =>
    retryAsyncWithBackOffAndTimeout(
      async () => {
        const paid = await getOdisPaymentsContract(client).read.totalPaidCUSD([account])
        // might replace bigNumber with big int but not yet
        return new BigNumber(paid.toString(10))
      },
      config.fullNodeRetryCount,
      [],
      config.fullNodeRetryDelayMs,
      undefined,
      config.fullNodeTimeoutMs,
    ).catch((err: any) => {
      logger.error({ err, account }, 'failed to get on-chain odis balance for account')
      Counters.blockchainErrors.inc()
      throw err
    }),
  )
}

export async function getDEK(client: Client, logger: Logger, account: Address): Promise<Hex> {
  const _meter = newMeter(Histograms.fullNodeLatency, 'getDataEncryptionKey')
  return _meter(() =>
    getDataEncryptionKey(
      account,
      client,
      logger,
      config.fullNodeTimeoutMs,
      config.fullNodeRetryCount,
      config.fullNodeRetryDelayMs,
    ).catch((err) => {
      logger.error({ err, account }, 'failed to get on-chain DEK for account')
      Counters.blockchainErrors.inc()
      throw err
    }),
  )
}
