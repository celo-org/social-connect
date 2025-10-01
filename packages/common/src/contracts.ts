import { accountsABI, odisPaymentsABI, stableTokenABI } from '@celo/abis'
import { Address, Client, getContract, GetContractReturnType } from 'viem'

import { celo, celoSepolia } from 'viem/chains'
const CONTRACTS: Record<number, Record<'accounts' | 'odisPayments' | 'cusd', Address>> = {
  [celo.id]: {
    accounts: '0x7d21685C17607338b313a7174bAb6620baD0aaB7',
    odisPayments: '0xae6b29f31b96e61dddc792f45fda4e4f0356d0cb',
    cusd: '0x765DE816845861e75A25fCA122bb6898B8B1282a',
  },
  [celoSepolia.id]: {
    accounts: '0x44957232699ca060B607E77083bDACD350d6b6d1',
    odisPayments: '0x96AfaE75F12A759c1dFB364ce93548c3Bd242D58',
    cusd: '0xEF4d55D6dE8e8d73232827Cd1e9b2F2dBb45bC80',
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
