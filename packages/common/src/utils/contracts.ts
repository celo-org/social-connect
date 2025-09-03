import {
  createWalletClient,
  http as viemHttpTransport,
  type HttpTransportConfig,
  type WalletClient,
} from 'viem'
import { celo, celoAlfajores } from 'viem/chains'

export interface BlockchainConfig {
  rpcURL: string
  chainID: number
  apiKey?: string
}

export function getWalletClient(config: BlockchainConfig): WalletClient {
  return createWalletClient({
    chain: config.chainID === celo.id ? celo : celoAlfajores,
    transport: viemHttpTransport(config.rpcURL, configureOptions(config, {})),
  })
}

export function getWalletClientWithAgent(config: BlockchainConfig): WalletClient {
  const options: HttpTransportConfig = {}

  options.fetchOptions = {}
  options.fetchOptions.keepalive = true

  return createWalletClient({
    chain: config.chainID === celo.id ? celo : celoAlfajores,
    transport: viemHttpTransport(config.rpcURL, configureOptions(config, options)),
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
