import { defineInt, defineString } from 'firebase-functions/params'
import * as functions from 'firebase-functions/v2/scheduler'
import { testDomainSignQuery, testPNPSignQuery } from './test'

const contextName = defineString('MONITOR_CONTEXT_NAME')
const blockchainProvider = defineString('BLOCKCHAIN_PROVIDER')
const chainID = defineInt('CHAIN_ID')

export const odisMonitorScheduleFunctionPNPGen2 = functions.onSchedule(
  'every 5 minutes',
  async () =>
    testPNPSignQuery(
      { rpcURL: blockchainProvider.value(), chainID: chainID.value() as 44787 },
      contextName.value() as any,
    ),
)

export const odisMonitorScheduleFunctionDomainsGen2 = functions.onSchedule(
  'every 5 minutes',
  async () => testDomainSignQuery(contextName.value() as any),
)
