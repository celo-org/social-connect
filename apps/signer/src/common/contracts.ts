import { retryAsyncWithBackOffAndTimeout } from '@celo/base'
import { getDataEncryptionKey, getOdisPaymentsContract } from '@celo/phone-number-privacy-common'
import { BigNumber } from 'bignumber.js'
import Logger from 'bunyan'
import { Address, Client, Hex } from 'viem'
import { config } from '../config'
import { Counters, Histograms, newMeter } from './metrics'

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
        return new BigNumber(paid.toString(10))
      },
      config.fullNodeRetryCount,
      [],
      config.fullNodeRetryDelayMs,
      undefined,
      config.fullNodeTimeoutMs,
    ).catch((err: any) => {
      logger.error(
        { err, account },
        `Error retrieving on-chain ODIS balance for account: ${account}. Please check network connectivity.`,
      )
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
      logger.error(
        { err, account },
        `Failed to retrieve Data Encryption Key (DEK) for account: ${account}. There may be an issue with blockchain access.`,
      )
      Counters.blockchainErrors.inc()
      throw err
    }),
  )
}
