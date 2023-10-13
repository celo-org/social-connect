import { ensureLeading0x } from '@celo/base'
import {
  FULL_NODE_TIMEOUT_IN_MS,
  RETRY_COUNT,
  RETRY_DELAY_IN_MS,
} from '@celo/phone-number-privacy-common'
import { defineBoolean, defineInt, defineSecret, defineString } from 'firebase-functions/params'

const defaultServiceName = 'odis-combiner'
export const defaultMockDEK = ensureLeading0x(
  'bf8a2b73baf8402f8fe906ad3f42b560bf14b39f7df7797ece9e293d6f162188'
)

// function settings
export const minInstancesConfig: any = defineInt('MIN_INSTANCES', { default: 0 })
export const requestConcurency: any = defineInt('REQUEST_CONCURRENCY', { default: 80 })

// Root
export const serviceNameConfig: any = defineString('SERVICE_NAME', { default: defaultServiceName })

// Blockchain
export const blockchainProvider: any = defineString('BLOCKCHAIN_PROVIDER')
export const blockchainApiKey: any = defineSecret('BLOCKCHAIN_API_KEY')

// PNP
export const pnpServiceName: any = defineString('PNP_SERVICE_NAME', { default: defaultServiceName })
export const pnpEnabled: any = defineBoolean('PNP_ENABLED', {
  default: false,
  description: '',
})
export const pnpOdisServicesSigners: any = defineString('PNP_ODIS_SERVICES_SIGNERS')
export const pnpOdisServicesTimeoutMilliseconds: any = defineInt(
  'PNP_ODIS_SERVICES_TIMEOUT_MILLISECONDS',
  {
    default: 5 * 1000,
  }
)
export const pnpKeysCurrentVersion: any = defineInt('PNP_KEYS_CURRENT_VERSION')
export const pnpKeysVersions: any = defineString('PNP_KEYS_VERSIONS')
export const pnpFullNodeTimeoutMs: any = defineInt('PNP_FULL_NODE_TIMEOUT_MS', {
  default: FULL_NODE_TIMEOUT_IN_MS,
})
export const pnpFullNodeRetryCount: any = defineInt('PNP_FULL_NODE_RETRY_COUNT', {
  default: RETRY_COUNT,
})
export const pnpFullNodeDelaysMs: any = defineInt('PNP_FULL_NODE_DELAY_MS', {
  default: RETRY_DELAY_IN_MS,
})
export const pnpShouldAuthenticate: any = defineBoolean('PNP_SHOULD_AUTHENTICATE', {
  default: true,
})
export const pnpShouldCheckQuota: any = defineBoolean('PNP_SHOULD_CHECK_QUOTA', {
  default: false,
})
export const pnpShouldMockAccountService: any = defineBoolean('PNP_SHOULD_MOCK_ACCOUNT_SERVICE', {
  default: false,
})
export const pnpMockDek: any = defineString('PNP_MOCK_DECK', { default: defaultMockDEK })

// Domains
export const domainServiceName: any = defineString('DOMAIN_SERVICE_NAME', {
  default: defaultServiceName,
})
export const domainEnabled: any = defineBoolean('DOMAIN_ENABLED', { default: false })
export const domainOdisServicesSigners: any = defineString('DOMAIN_ODIS_SERVICES_SIGNERS')
export const domainOdisServicesTimeoutMilliseconds: any = defineInt(
  'DOMAIN_ODIS_SERVICES_TIMEOUT_MILLISECONDS',
  {
    default: 5 * 1000,
  }
)
export const domainKeysCurrentVersion: any = defineInt('DOMAIN_KEYS_CURRENT_VERSION')
export const domainKeysVersions: any = defineString('DOMAIN_KEYS_VERSIONS')
export const domainFullNodeTimeoutMs: any = defineInt('DOMAIN_FULL_NODE_TIMEOUT_MS', {
  default: FULL_NODE_TIMEOUT_IN_MS,
})
export const domainFullNodeRetryCount: any = defineInt('DOMAIN_FULL_NODE_RETRY_COUNT', {
  default: RETRY_COUNT,
})
export const domainFullNodeDelaysMs: any = defineInt('DOMAIN_FULL_NODE_DELAY_MS', {
  default: RETRY_DELAY_IN_MS,
})
export const domainShouldAuthenticate: any = defineBoolean('DOMAIN_SHOULD_AUTHENTICATE', {
  default: true,
})
export const domainShouldCheckQuota: any = defineBoolean('DOMAIN_SHOULD_CHECK_QUOTA', {
  default: false,
})
