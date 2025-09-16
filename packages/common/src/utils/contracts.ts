import {
  createWalletClient,
  http as viemHttpTransport,
  type HttpTransportConfig,
  type WalletClient,
} from 'viem'
import { celo, celoSepolia } from 'viem/chains'

export interface BlockchainConfig {
  rpcURL: string
  chainID: number
  apiKey?: string
}

function getChainFromId(chainID: number) {
  switch (chainID) {
    case celo.id:
      return celo
    case celoSepolia.id:
      return celoSepolia
    default:
      // Default to Celo Sepolia for backward compatibility
      return celoSepolia
  }
}

export function getWalletClient(config: BlockchainConfig): WalletClient {
  return createWalletClient({
    chain: getChainFromId(config.chainID),
    transport: viemHttpTransport(config.rpcURL, configureOptions(config, {})),
  })
}

export function getWalletClientWithAgent(config: BlockchainConfig): WalletClient {
  const options: HttpTransportConfig = {}

  options.fetchOptions = {}
  options.fetchOptions.keepalive = true

  return createWalletClient({
    chain: getChainFromId(config.chainID),
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
