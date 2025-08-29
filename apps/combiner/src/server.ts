import { ContractKit } from '@celo/contractkit'
import {
  CombinerEndpoint,
  getContractKitWithAgent,
  KEY_VERSION_HEADER,
  loggerMiddleware,
  OdisRequest,
  rootLogger,
} from '@celo/phone-number-privacy-common'
import express, { RequestHandler } from 'express'
import fs from 'fs'
import https from 'https'
import { Signer } from './common/combine'
import {
  catchErrorHandler,
  disabledHandler,
  Locals,
  meteringHandler,
  resultHandler,
  ResultHandler,
  tracingHandler,
} from './common/handlers'
import { Histograms, register } from './common/metrics'
import { CombinerConfig, getCombinerVersion } from './config'
import { disableDomain } from './domain/endpoints/disable/action'
import { domainQuota } from './domain/endpoints/quota/action'
import { domainSign } from './domain/endpoints/sign/action'
import { pnpQuota } from './pnp/endpoints/quota/action'
import { pnpSign } from './pnp/endpoints/sign/action'
import {
  CachingAccountService,
  ContractKitAccountService,
  MockAccountService,
} from './pnp/services/account-services'
import { NoQuotaCache } from './utils/no-quota-cache'

require('events').EventEmitter.defaultMaxListeners = 15

export function startCombiner(config: CombinerConfig, kit?: ContractKit) {
  const logger = rootLogger(config.serviceName)
  kit = kit ?? getContractKitWithAgent(config.blockchain)

  logger.info('Creating combiner express server')
  const app = express()

  app.use(express.json({ limit: '0.2mb' }) as RequestHandler, loggerMiddleware(config.serviceName))

  // Enable cross origin resource sharing from any domain
  app.use((req, res, next) => {
    res.header('Access-Control-Allow-Origin', '*')
    res.header('Access-Control-Allow-Methods', 'GET, POST, OPTIONS')
    res.header(
      'Access-Control-Allow-Headers',
      `Origin, X-Requested-With, Content-Type, Accept, Authorization, ${KEY_VERSION_HEADER}`,
    )
    if (req.method === 'OPTIONS') {
      return res.sendStatus(200)
    }
    next()
  })

  app.get(CombinerEndpoint.STATUS, (_req, res) => {
    res.status(200).json({
      version: getCombinerVersion(),
    })
  })

  const baseAccountService = config.phoneNumberPrivacy.shouldMockAccountService
    ? new MockAccountService(config.phoneNumberPrivacy.mockDek!)
    : new ContractKitAccountService(logger, kit)

  const accountService = new CachingAccountService(baseAccountService)
  const noQuotaCache = new NoQuotaCache()

  let pnpSigners: Signer[] = []
  let domainSigners: Signer[] = []
  try {
    pnpSigners = JSON.parse(config.phoneNumberPrivacy.odisServices.signers)
    domainSigners = JSON.parse(config.domains.odisServices.signers)
  } catch (error) {
    logger.error('Failed to parse ODIS signer configs', error)
    throw new Error('Invalid signer configuration')
  }

  const { domains, phoneNumberPrivacy } = config

  app.post(
    CombinerEndpoint.PNP_QUOTA,
    createHandler(
      phoneNumberPrivacy.enabled,
      pnpQuota(pnpSigners, config.phoneNumberPrivacy, accountService, noQuotaCache),
    ),
  )
  app.post(
    CombinerEndpoint.PNP_SIGN,
    createHandler(
      phoneNumberPrivacy.enabled,
      pnpSign(pnpSigners, config.phoneNumberPrivacy, accountService, noQuotaCache),
    ),
  )
  app.post(
    CombinerEndpoint.DOMAIN_QUOTA_STATUS,
    createHandler(domains.enabled, domainQuota(domainSigners, config.domains)),
  )
  app.post(
    CombinerEndpoint.DOMAIN_SIGN,
    createHandler(domains.enabled, domainSign(domainSigners, domains)),
  )
  app.post(
    CombinerEndpoint.DISABLE_DOMAIN,
    createHandler(domains.enabled, disableDomain(domainSigners, domains)),
  )
  app.get(CombinerEndpoint.METRICS, (_req, res) => {
    res.send(register.metrics())
  })

  const sslOptions = getSslOptions(config)
  if (sslOptions) {
    logger.info('Starting HTTPS server...')
    return https.createServer(sslOptions, app)
  }

  logger.info('Starting HTTP server...')
  return app
}

function getSslOptions(config: CombinerConfig) {
  const logger = rootLogger(config.serviceName)
  const { sslKeyPath, sslCertPath } = config.server

  if (!sslKeyPath || !sslCertPath) {
    logger.info('No SSL configs specified')
    return null
  }

  if (!fs.existsSync(sslKeyPath) || !fs.existsSync(sslCertPath)) {
    logger.error('SSL cert files not found')
    return null
  }

  return {
    key: fs.readFileSync(sslKeyPath),
    cert: fs.readFileSync(sslCertPath),
  }
}

function createHandler<R extends OdisRequest>(
  enabled: boolean,
  action: ResultHandler<R>,
): RequestHandler<{}, {}, R, {}, Locals> {
  return catchErrorHandler(
    tracingHandler(
      meteringHandler(
        Histograms.responseLatency,
        enabled ? resultHandler(action) : disabledHandler,
      ),
    ),
  )
}
