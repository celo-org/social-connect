import { OdisContextName } from '@celo/identity/lib/odis/query'
import { CombinerEndpointPNP, rootLogger } from '@celo/phone-number-privacy-common'
import { Hex } from 'viem'
import yargs from 'yargs'
import { concurrentRPSLoadTest, type TestChainInfo } from '../test'

const logger = rootLogger('odis-monitor')

yargs
  .scriptName('ODIS-load-test')
  .recommendCommands()
  .demandCommand(1)
  .strict(true)
  .showHelpOnFail(true)
  .command(
    'run <contextName> <rps>',
    'Load test ODIS.',
    (args) =>
      args
        .positional('contextName', {
          type: 'string',
          description: 'Desired network.',
        })
        .positional('rps', {
          type: 'number',
          description: 'Number of requests per second to generate',
        })
        .option('duration', {
          type: 'number',
          description: 'Duration of the loadtest in Ms.',
          default: 0,
        })
        .option('bypassQuota', {
          type: 'boolean',
          description: 'Bypass Signer quota check.',
          default: false,
        })
        .option('useDEK', {
          type: 'boolean',
          description: 'Use Data Encryption Key (DEK) to authenticate.',
          default: false,
        })
        .option('movingAvgRequests', {
          type: 'number',
          description: 'number of requests to use when calculating latency moving average',
          default: 50,
        })
        .option('privateKey', {
          type: 'string',
          description: 'optional private key to send requests from',
        })
        .option('privateKeyPercentage', {
          type: 'number',
          description: 'percentage of time to use privateKey, if specified',
          default: 100,
        }),
    (args) => {
      if (args.rps == null || args.contextName == null) {
        logger.error('missing positional arguments')
        yargs.showHelp()
        process.exit(1)
      }
      const rps = args.rps!
      const contextName = args.contextName! as OdisContextName

      let blockchainProvider: TestChainInfo
      switch (contextName) {
        case 'alfajoresstaging':
        case 'alfajores':
          blockchainProvider = {
            rpcURL: 'https://alfajores-forno.celo-testnet.org',
            chainID: 44787,
          }
          break
        case 'mainnet':
          blockchainProvider = { rpcURL: 'https://forno.celo.org', chainID: 42220 }
          break
        default:
          logger.error('Invalid contextName')
          yargs.showHelp()
          process.exit(1)
      }

      if (rps < 1) {
        logger.error('Invalid rps')
        yargs.showHelp()
        process.exit(1)
      }
      concurrentRPSLoadTest(
        args.rps,
        blockchainProvider,
        contextName,
        CombinerEndpointPNP.PNP_SIGN,
        args.duration,
        args.bypassQuota,
        args.useDEK,
        args.movingAvgRequests,
        args.privateKey as Hex,
        args.privateKeyPercentage,
      )
    },
  ).argv
