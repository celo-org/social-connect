import { ErrorMessage } from '@celo/phone-number-privacy-common'
import Logger from 'bunyan'
import { LRUCache } from 'lru-cache'
import { Address, Client } from 'viem'
import { getDEK } from '../../common/contracts'
import { OdisError, wrapError } from '../../common/error'
import { Counters } from '../../common/metrics'
import { traceAsyncFunction } from '../../common/tracing-utils'

export interface AccountService {
  getAccount(address: string): Promise<string>
}

export interface ViemAccountServiceOptions {
  fullNodeTimeoutMs: number
  fullNodeRetryCount: number
  fullNodeRetryDelayMs: number
}

export class CachingAccountService implements AccountService {
  private cache: LRUCache<string, string, any>
  constructor(baseService: AccountService) {
    this.cache = new LRUCache({
      max: 500,
      ttl: 5 * 1000, // 5 seconds
      allowStale: true,
      fetchMethod: async (address: string) => {
        return baseService.getAccount(address)
      },
    })
  }

  getAccount = (address: string): Promise<string> => {
    return traceAsyncFunction('CachingAccountService - getAccount', async () => {
      const dek = await this.cache.fetch(address)

      if (dek === undefined) {
        Counters.errors.labels('NA', ErrorMessage.FAILURE_TO_GET_DEK).inc()
        throw new OdisError(ErrorMessage.FAILURE_TO_GET_DEK)
      }
      return dek
    })
  }
}

// tslint:disable-next-line:max-classes-per-file
export class ViemAccountService implements AccountService {
  constructor(
    private readonly logger: Logger,
    private readonly client: Client,
  ) {}

  async getAccount(address: Address): Promise<string> {
    return traceAsyncFunction('ViemAccountService - getAccount', async () => {
      return wrapError(getDEK(this.client, this.logger, address), ErrorMessage.FAILURE_TO_GET_DEK)
    })
  }
}

// tslint:disable-next-line:max-classes-per-file
export class MockAccountService implements AccountService {
  constructor(private readonly mockDek: string) {}

  async getAccount(_address: string): Promise<string> {
    return this.mockDek
  }
}
