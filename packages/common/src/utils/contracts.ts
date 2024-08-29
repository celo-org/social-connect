import {
  createWalletClient,
  http as viemHttpTransport,
  type HttpTransportConfig,
  type WalletClient,
} from 'viem'
import { celo } from 'viem/chains'

export interface BlockchainConfig {
  provider: string
  apiKey?: string
}

export function getWalletClient(config: BlockchainConfig): WalletClient {
  return createWalletClient({
    chain: celo,
    transport: viemHttpTransport(config.provider, configureOptions(config, {})),
  })
}

export function getWalletClientWithAgent(config: BlockchainConfig): WalletClient {
  const options: HttpTransportConfig = {}

  options.fetchOptions = {}
  options.fetchOptions.keepalive = true

  // no agent on viem?
  // options.fetchOptions = {
  //   http: new http.Agent({ keepAlive: true }),
  //   https: new https.Agent({ keepAlive: true }),
  // }

  return createWalletClient({
    chain: celo,
    transport: viemHttpTransport(config.provider, configureOptions(config, options)),
  })
}

function configureOptions(
  config: BlockchainConfig,
  options: HttpTransportConfig,
): HttpTransportConfig {
  if (config.apiKey) {
    options.fetchOptions ||= {}
    const headers = options.fetchOptions.headers || {}
    options.fetchOptions.headers = { ...headers, apiKey: config.apiKey }
  }
  return options
}
