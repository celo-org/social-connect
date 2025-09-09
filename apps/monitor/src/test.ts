import { sleep } from '@celo/base'
import { Result } from '@celo/base/lib/result'
import { BackupError } from '@celo/encrypted-backup'
import { IdentifierHashDetails } from '@celo/identity/lib/odis/identifier'
import { ErrorMessages, OdisContextName } from '@celo/identity/lib/odis/query'
import { PnpClientQuotaStatus } from '@celo/identity/lib/odis/quota'
import { CombinerEndpointPNP, rootLogger } from '@celo/phone-number-privacy-common'
import { performance } from 'perf_hooks'
import { Hex } from 'viem'
import { ChainInfo, queryOdisDomain, queryOdisForQuota, queryOdisForSalt } from './query'

const getLogger = () => rootLogger('odis-monitor')

export async function testPNPSignQuery(
  blockchainProvider: ChainInfo,
  contextName: OdisContextName,
  timeoutMs?: number,
  bypassQuota?: boolean,
  useDEK?: boolean,
  privateKey?: string,
  privateKeyPercentage: number = 100,
) {
  getLogger().debug(
    { blockchainProvider, contextName, bypassQuota, useDEK },
    'Starting PNP sign query test',
  )
  try {
    const odisResponse: IdentifierHashDetails = await queryOdisForSalt(
      blockchainProvider,
      contextName,
      timeoutMs,
      bypassQuota,
      useDEK,
      privateKey,
      privateKeyPercentage,
    )
    getLogger().debug({ odisResponse }, 'ODIS salt request successful. System is healthy.')
  } catch (err) {
    if ((err as Error).message === ErrorMessages.ODIS_QUOTA_ERROR) {
      getLogger().warn(
        { error: err },
        'ODIS salt request out of quota. This is expected. System is healthy.',
      )
    } else {
      getLogger().error('ODIS salt request failed.')
      getLogger().error({ err })
      throw err
    }
  }
}

export async function testPNPQuotaQuery(
  blockchainProvider: ChainInfo,
  contextName: OdisContextName,
  timeoutMs?: number,
  privateKey?: Hex,
  privateKeyPercentage: number = 100,
) {
  getLogger().info(`Performing test PNP query for ${CombinerEndpointPNP.PNP_QUOTA}`)
  try {
    const odisResponse: PnpClientQuotaStatus = await queryOdisForQuota(
      blockchainProvider,
      contextName,
      timeoutMs,
      privateKey,
      privateKeyPercentage,
    )
    getLogger().info({ odisResponse }, 'ODIS quota request successful. System is healthy.')
  } catch (err) {
    getLogger().error('ODIS quota request failed.')
    getLogger().error({ err })
    throw err
  }
}

export async function testDomainSignQuery(contextName: OdisContextName) {
  getLogger().info('Performing test domains query')
  let odisResponse: Result<Buffer, BackupError>
  try {
    odisResponse = await queryOdisDomain(contextName)
    getLogger().info({ odisResponse }, 'ODIS response')
  } catch (err) {
    getLogger().error('ODIS key hardening request failed.')
    getLogger().error({ err })
    throw err
  }
  if (odisResponse.ok) {
    getLogger().info('System is healthy')
  } else {
    throw new Error('Received not ok response')
  }
}

export async function concurrentRPSLoadTest(
  rps: number,
  blockchainProvider: ChainInfo,
  contextName: OdisContextName,
  endpoint:
    | CombinerEndpointPNP.PNP_QUOTA
    | CombinerEndpointPNP.PNP_SIGN = CombinerEndpointPNP.PNP_SIGN,
  duration: number = 0,
  bypassQuota: boolean = false,
  useDEK: boolean = false,
  movingAverageRequests: number = 50,
  privateKey?: Hex,
  privateKeyPercentage: number = 100,
) {
  const latencyQueue: number[] = []
  let movingAvgLatencySum = 0
  let latencySum = 0
  let index = 1

  function measureLatency(fn: () => Promise<void>): () => Promise<void> {
    return async () => {
      const start = performance.now()

      await fn()

      const reqLatency = performance.now() - start
      latencySum += reqLatency
      movingAvgLatencySum += reqLatency

      const queuelength = latencyQueue.push(reqLatency)
      if (queuelength > movingAverageRequests) {
        movingAvgLatencySum -= latencyQueue.shift()!
      }

      const stats = {
        averageLatency: Math.round(latencySum / index),
        movingAverageLatency: Math.round(movingAvgLatencySum / latencyQueue.length),
        index,
      }

      if (reqLatency > 600) {
        getLogger().warn(stats, 'SLOW Request')
      } else {
        getLogger().info(stats, 'request finished')
      }
      index++
    }
  }

  const testFn = async () => {
    try {
      await (endpoint === CombinerEndpointPNP.PNP_SIGN
        ? testPNPSignQuery(
            blockchainProvider,
            contextName,
            undefined,
            bypassQuota,
            useDEK,
            privateKey,
            privateKeyPercentage,
          )
        : testPNPQuotaQuery(
            blockchainProvider,
            contextName,
            undefined,
            privateKey,
            privateKeyPercentage,
          ))
    } catch {
      getLogger().error('load test request failed')
    }
  }

  return doRPSTest(measureLatency(testFn), rps, duration)
}

async function doRPSTest(
  testFn: () => Promise<void>,
  rps: number,
  duration: number = 0,
): Promise<void> {
  const inFlightRequests: Array<Promise<void>> = []
  let shouldRun = true

  async function requestSender() {
    while (shouldRun) {
      for (let i = 0; i < rps; i++) {
        inFlightRequests.push(testFn())
      }
      await sleep(1000)
    }
  }

  async function requestEnder() {
    while (shouldRun || inFlightRequests.length > 0) {
      if (inFlightRequests.length > 0) {
        const req = inFlightRequests.shift()
        await req?.catch((_err) => {
          getLogger().error('load test request failed')
        })
      } else {
        await sleep(1000)
      }
    }
  }

  async function durationChecker() {
    await sleep(duration)
    shouldRun = false
  }

  if (duration === 0) {
    await Promise.all([requestSender(), requestEnder()])
  } else {
    await Promise.all([durationChecker(), requestSender(), requestEnder()])
  }
}
