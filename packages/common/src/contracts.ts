import { accountsABI, odisPaymentsABI, stableTokenABI } from '@celo/abis'
import { Address, Client, getContract, GetContractReturnType } from 'viem'

import { celo, celoAlfajores } from 'viem/chains'
const CONTRACTS: Record<number, Record<'accounts' | 'odisPayments' | 'cusd', Address>> = {
  [celo.id]: {
    accounts: '0x7d21685C17607338b313a7174bAb6620baD0aaB7',
    odisPayments: '0xae6b29f31b96e61dddc792f45fda4e4f0356d0cb',
    cusd: '0x765DE816845861e75A25fCA122bb6898B8B1282a',
  },
  [celoAlfajores.id]: {
    accounts: '0xed7f51A34B4e71fbE69B3091FcF879cD14bD73A9',
    odisPayments: '0x645170cdB6B5c1bc80847bb728dBa56C50a20a49',
    cusd: '0x874069Fa1Eb16D44d622F2e0Ca25eeA172369bC1',
  },
}

export const getAccountsContract: <TClient extends Client>(
  client: TClient,
) => GetContractReturnType<typeof accountsABI, TClient> = (client) =>
  getContract({
    abi: accountsABI,
    client,
    address: getAddresses(client.chain?.id as number).accounts,
  })

export const getOdisPaymentsContract: <TClient extends Client>(
  client: TClient,
) => GetContractReturnType<typeof odisPaymentsABI, Client> = (client) =>
  getContract({
    abi: odisPaymentsABI,
    client,
    address: getAddresses(client.chain?.id as number).odisPayments,
  })

export const getCUSDContract: <TClient extends Client>(
  client: TClient,
) => GetContractReturnType<typeof stableTokenABI, Client> = (client) =>
  getContract({
    abi: stableTokenABI,
    client,
    address: getAddresses(client.chain?.id as number).cusd,
  })

function getAddresses(chainId: number) {
  return CONTRACTS[chainId]
}
