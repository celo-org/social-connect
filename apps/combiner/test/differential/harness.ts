// Runs the TS combiner and the Rust combiner side by side, each in front of its own three TS
// signers, and sends both the same raw requests.
//
// Each combiner has its own signers because a request sent through both combiners to shared
// signers would reach every signer twice, and the second time it is a replay.
import {
  DB_TIMEOUT,
  FULL_NODE_TIMEOUT_IN_MS,
  getAccountsContract,
  getOdisPaymentsContract,
  RETRY_COUNT,
  RETRY_DELAY_IN_MS,
  TestUtils,
} from '@celo/phone-number-privacy-common'
import { initDatabase as initSignerDatabase } from '@celo/phone-number-privacy-signer/dist/common/database/database'
import { DefaultKeyName } from '@celo/phone-number-privacy-signer/dist/common/key-management/key-provider-base'
import { MockKeyProvider } from '@celo/phone-number-privacy-signer/dist/common/key-management/mock-key-provider'
import {
  SignerConfig,
  SupportedDatabase,
  SupportedKeystore,
} from '@celo/phone-number-privacy-signer/dist/config'
import { startSigner } from '@celo/phone-number-privacy-signer/dist/server'
import BigNumber from 'bignumber.js'
import { ChildProcess, spawn } from 'child_process'
import { Express } from 'express'
import http from 'http'
import { Knex } from 'knex'
import { AddressInfo } from 'net'
import os from 'os'
import path from 'path'
import {
  Address,
  createWalletClient,
  decodeFunctionData,
  encodeFunctionResult,
  Hex,
  http as httpTransport,
  isAddressEqual,
  keccak256,
  publicActions,
  stringToHex,
  toHex,
} from 'viem'
import { privateKeyToAddress } from 'viem/accounts'
import { celoSepolia } from 'viem/chains'
import config from '../../src/config'
import { startCombiner } from '../../src/server'

const {
  DEK_PUBLIC_KEY,
  PNP_THRESHOLD_DEV_PK_SHARE_1_V1,
  PNP_THRESHOLD_DEV_PK_SHARE_1_V2,
  PNP_THRESHOLD_DEV_PK_SHARE_1_V3,
  PNP_THRESHOLD_DEV_PK_SHARE_2_V1,
  PNP_THRESHOLD_DEV_PK_SHARE_2_V2,
  PNP_THRESHOLD_DEV_PK_SHARE_2_V3,
  PNP_THRESHOLD_DEV_PK_SHARE_3_V1,
  PNP_THRESHOLD_DEV_PK_SHARE_3_V2,
  PNP_THRESHOLD_DEV_PK_SHARE_3_V3,
} = TestUtils.Values

/** The headers an answer is compared on. */
const COMPARED_HEADERS = [
  'access-control-allow-origin',
  'access-control-allow-methods',
  'access-control-allow-headers',
  'allow',
] as const

export interface RawRequest {
  method: string
  path: string
  /** Sent as given; an array sends the header once per value. */
  headers: http.OutgoingHttpHeaders
  body: Buffer
}

export interface Answer {
  status: number
  /**
   * A JSON body written back with `JSON.stringify`, key order kept, and its `version` replaced
   * by `"<version>"`, since each combiner reports its own. Any other body as text.
   */
  body: string
  /** The headers of COMPARED_HEADERS that are present, and the media type of `content-type`. */
  headers: Record<string, string>
}

export interface Harness {
  send(request: RawRequest): Promise<{ ts: Answer; rust: Answer }>
  /** What the Rust combiner has written to stdout and stderr so far. */
  rustCombinerLog(): string
  stop(): Promise<void>
}

export interface TestUser {
  privateKey: Hex
  /** In lower case, as the SDK's tests write accounts. */
  account: Address
}

/** A wallet key and account of its own for every label. */
export function testUser(label: string): TestUser {
  const privateKey = keccak256(stringToHex(`odis differential test: ${label}`))
  return { privateKey, account: privateKeyToAddress(privateKey).toLowerCase() as Address }
}

/** The one account without a DEK; every other account has DEK_PUBLIC_KEY. */
export const USER_WITHOUT_DEK = testUser('without a DEK')
/** The one account that has paid for no queries; every other account has paid for 1000. */
export const USER_WITHOUT_QUOTA = testUser('without quota')

// The chain state both kinds of lookup read: the TS services through their viem client, the Rust
// combiner through the stub full node. Both answer from these functions, so they cannot differ.

function registeredDek(account: Address): Hex {
  return isAddressEqual(account, USER_WITHOUT_DEK.account) ? '0x' : (DEK_PUBLIC_KEY as Hex)
}

/** cUSD paid to OdisPayments, in wei. At 0.001 cUSD per query, 1e18 buys 1000 queries. */
function totalPaidCusd(account: Address): bigint {
  return isAddressEqual(account, USER_WITHOUT_QUOTA.account) ? 0n : 10n ** 18n
}

const viemClient = createWalletClient({ chain: celoSepolia, transport: httpTransport() })
  .extend(publicActions)
  .extend(() => ({
    readContract: async ({ address, functionName, args }: any) => {
      if (isAddressEqual(address, accountsContract.address)) {
        if (functionName === 'getDataEncryptionKey') {
          return registeredDek(args[0])
        }
      }
      if (isAddressEqual(address, odisPaymentsContract.address)) {
        if (functionName === 'totalPaidCUSD') {
          return totalPaidCusd(args[0])
        }
      }
      throw new Error(`Unmocked contract call: ${functionName} on ${address}`)
    },
  }))
const accountsContract = getAccountsContract(viemClient)
const odisPaymentsContract = getOdisPaymentsContract(viemClient)

const signerConfig: SignerConfig = {
  serviceName: 'odis-signer',
  server: { port: 0, sslKeyPath: undefined, sslCertPath: undefined },
  quota: { queryPriceInCUSD: new BigNumber(0.001) },
  api: { domains: { enabled: false }, phoneNumberPrivacy: { enabled: true } },
  blockchain: {
    rpcURL: 'https://forno.celo-sepolia.celo-testnet.org',
    chainID: celoSepolia.id,
    apiKey: undefined,
  },
  db: {
    type: SupportedDatabase.Sqlite,
    user: '',
    password: '',
    database: '',
    host: 'http://localhost',
    port: undefined,
    ssl: true,
    poolMaxSize: 50,
    timeout: DB_TIMEOUT,
  },
  keystore: {
    type: SupportedKeystore.MOCK_SECRET_MANAGER,
    keys: {
      phoneNumberPrivacy: { name: 'phoneNumberPrivacy', latest: 2 },
      domains: { name: 'domains', latest: 1 },
    },
    azure: { clientID: '', clientSecret: '', tenant: '', vaultName: '' },
    google: { projectId: '' },
    aws: { region: '', secretKey: '' },
  },
  timeout: 5000,
  test_quota_bypass_percentage: 0,
  fullNodeTimeoutMs: FULL_NODE_TIMEOUT_IN_MS,
  fullNodeRetryCount: RETRY_COUNT,
  fullNodeRetryDelayMs: RETRY_DELAY_IN_MS,
  shouldMockAccountService: false,
  mockDek: '',
  mockTotalQuota: 0,
  shouldMockRequestService: false,
  requestPrunningDays: 0,
  requestPrunningAtServerStart: false,
  requestPrunningJobCronPattern: '0 0 0 * * *',
}

/** The dev key shares 1 to 3 of key versions 1 to 3, one list per signer. */
const KEY_SHARES = [
  [
    PNP_THRESHOLD_DEV_PK_SHARE_1_V1,
    PNP_THRESHOLD_DEV_PK_SHARE_1_V2,
    PNP_THRESHOLD_DEV_PK_SHARE_1_V3,
  ],
  [
    PNP_THRESHOLD_DEV_PK_SHARE_2_V1,
    PNP_THRESHOLD_DEV_PK_SHARE_2_V2,
    PNP_THRESHOLD_DEV_PK_SHARE_2_V3,
  ],
  [
    PNP_THRESHOLD_DEV_PK_SHARE_3_V1,
    PNP_THRESHOLD_DEV_PK_SHARE_3_V2,
    PNP_THRESHOLD_DEV_PK_SHARE_3_V3,
  ],
]

const SIGNER_MIGRATIONS = path.join(
  path.dirname(require.resolve('@celo/phone-number-privacy-signer/package.json')),
  'dist/common/database/migrations',
)

const LOCALHOST = '127.0.0.1'

function listen(server: http.Server): Promise<string> {
  return new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, LOCALHOST, () => {
      const { port } = server.address() as AddressInfo
      resolve(`http://${LOCALHOST}:${port}`)
    })
  })
}

async function close(server: http.Server): Promise<void> {
  await new Promise((resolve) => {
    server.close(resolve)
    server.closeAllConnections()
  })
}

/** Everything the harness has started, so that `stop()` or a failed start ends all of it. */
interface Started {
  servers: http.Server[]
  databases: Knex[]
  rustCombiner?: ChildProcess
  /** The Rust combiner's stdout and stderr, in the order they arrived. */
  rustCombinerOutput: string[]
}

/** Listens on localhost and records the server in `started`. */
async function serve(started: Started, server: http.Server): Promise<string> {
  const url = await listen(server)
  started.servers.push(server)
  return url
}

async function stopAll(started: Started): Promise<void> {
  if (started.rustCombiner) {
    await stopChild(started.rustCombiner)
  }
  for (const server of [...started.servers].reverse()) {
    await close(server)
  }
  for (const database of started.databases) {
    await database.destroy()
  }
}

/** Three TS signers with the dev shares 1 to 3, each with a fresh SQLite database. */
async function startSigners(started: Started): Promise<string[]> {
  const urls: string[] = []
  for (const shares of KEY_SHARES) {
    const keyProvider = new MockKeyProvider(
      new Map(shares.map((share, i) => [`${DefaultKeyName.PHONE_NUMBER_PRIVACY}-${i + 1}`, share])),
    )
    const database = await initSignerDatabase(signerConfig, SIGNER_MIGRATIONS)
    started.databases.push(database)
    const app = startSigner(signerConfig, database, keyProvider, viemClient) as Express
    urls.push(await serve(started, http.createServer(app)))
  }
  return urls
}

function readBody(stream: http.IncomingMessage): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = []
    stream.on('data', (chunk: Buffer) => chunks.push(chunk))
    stream.on('end', () => resolve(Buffer.concat(chunks)))
    stream.on('error', reject)
  })
}

/**
 * A JSON-RPC server that answers `eth_chainId` with Celo Sepolia's chain ID and the Accounts
 * contract's `getDataEncryptionKey` from `registeredDek`.
 */
function fullNode(): http.Server {
  return http.createServer(async (req, res) => {
    const call = JSON.parse((await readBody(req)).toString('utf8'))
    const answer = (fields: object) => {
      res.setHeader('content-type', 'application/json')
      res.end(JSON.stringify({ jsonrpc: '2.0', id: call.id, ...fields }))
    }
    if (call.method === 'eth_chainId') {
      return answer({ result: toHex(celoSepolia.id) })
    }
    if (call.method === 'eth_call') {
      const [{ to, input, data }] = call.params
      if (isAddressEqual(to, accountsContract.address)) {
        const decoded = decodeFunctionData({ abi: accountsContract.abi, data: input ?? data })
        if (decoded.functionName === 'getDataEncryptionKey') {
          const [account] = decoded.args as [Address]
          return answer({
            result: encodeFunctionResult({
              abi: accountsContract.abi,
              functionName: 'getDataEncryptionKey',
              result: registeredDek(account),
            }),
          })
        }
      }
    }
    answer({
      error: { code: -32601, message: `the stub full node does not answer ${call.method}` },
    })
  })
}

async function freePort(): Promise<number> {
  const server = http.createServer()
  const url = await listen(server)
  await close(server)
  return Number(new URL(url).port)
}

/** The Rust combiner binary named by ODIS_RUST_COMBINER_BIN, as an absolute path. */
function rustCombinerBinary(): string {
  const binary = process.env.ODIS_RUST_COMBINER_BIN
  if (!binary) {
    throw new Error(
      'ODIS_RUST_COMBINER_BIN is not set. Run `just test-differential`, which builds the Rust combiner and sets it.',
    )
  }
  return path.resolve(binary)
}

/**
 * Starts `binary` in front of `signers`, records it in `started`, and returns its URL once it
 * answers `/status`. The environment holds only the combiner's settings, so nothing from the
 * shell, such as a proxy variable, reaches it, and the working directory has no `.env` for it to
 * read.
 */
async function startRustCombiner(
  started: Started,
  binary: string,
  signers: string[],
  fullNodeUrl: string,
): Promise<string> {
  const port = await freePort()
  const child = spawn(binary, [], {
    cwd: os.tmpdir(),
    env: {
      SERVER_PORT: String(port),
      PNP_ODIS_SERVICES_SIGNERS: JSON.stringify(
        signers.map((url, shareIndex) => ({ url, shareIndex })),
      ),
      PNP_KEYS_VERSIONS: config.phoneNumberPrivacy.keys.versions,
      PNP_KEYS_CURRENT_VERSION: '1',
      CHAIN_ID: String(celoSepolia.id),
      LOG_FORMAT: 'json',
      BLOCKCHAIN_PROVIDER: fullNodeUrl,
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  })
  started.rustCombiner = child

  const output = started.rustCombinerOutput
  child.stdout!.on('data', (chunk: Buffer) => output.push(chunk.toString('utf8')))
  child.stderr!.on('data', (chunk: Buffer) => output.push(chunk.toString('utf8')))
  const url = `http://${LOCALHOST}:${port}`

  const exited = new Promise<never>((_, reject) => {
    // A failed spawn, such as a missing binary, is an `error` event without `close`.
    child.on('error', (error) =>
      reject(new Error(`the Rust combiner could not be started: ${error.message}`)),
    )
    // `close` comes after the output streams have ended, so the error carries all of it.
    child.once('close', (code, signal) =>
      reject(
        new Error(
          `the Rust combiner exited (code ${code}, signal ${signal}) before it answered /status:\n${output.join('')}`,
        ),
      ),
    )
  })
  exited.catch(() => undefined)
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    const status = await Promise.race([
      fetch(`${url}/status`).then(
        (res) => res.status,
        () => undefined,
      ),
      exited,
    ])
    if (status === 200) {
      return url
    }
    await new Promise((resolve) => setTimeout(resolve, 100))
  }
  throw new Error(`the Rust combiner did not answer /status within 30 s:\n${output.join('')}`)
}

async function stopChild(child: ChildProcess): Promise<void> {
  // No pid: the spawn failed, so there is no process to stop.
  if (child.pid === undefined || child.exitCode !== null || child.signalCode !== null) {
    return
  }
  const exited = new Promise((resolve) => child.once('exit', resolve))
  child.kill('SIGTERM')
  const timer = setTimeout(() => child.kill('SIGKILL'), 5_000)
  await exited
  clearTimeout(timer)
}

function maskVersion(text: string): string {
  let parsed: unknown
  try {
    parsed = JSON.parse(text)
  } catch {
    return text
  }
  if (parsed !== null && typeof parsed === 'object' && 'version' in parsed) {
    ;(parsed as Record<string, unknown>).version = '<version>'
  }
  return JSON.stringify(parsed)
}

/** Sends the request's exact bytes on a connection of its own. */
function sendRaw(baseUrl: string, request: RawRequest): Promise<Answer> {
  const { hostname, port } = new URL(baseUrl)
  return new Promise((resolve, reject) => {
    const req = http.request({
      host: hostname,
      port,
      method: request.method,
      path: request.path,
      headers: request.headers,
      agent: false,
    })
    let answered = false
    req.on('response', (res) => {
      answered = true
      const headers: Record<string, string> = {}
      for (const name of COMPARED_HEADERS) {
        const value = res.headers[name]
        if (value !== undefined) {
          headers[name] = String(value)
        }
      }
      const contentType = res.headers['content-type']
      if (contentType !== undefined) {
        headers['content-type'] = contentType.split(';')[0].trim()
      }
      readBody(res).then(
        (body) =>
          resolve({ status: res.statusCode!, body: maskVersion(body.toString('utf8')), headers }),
        reject,
      )
    })
    // A server may answer before it has read the whole body (413) and close the connection
    // while the body is still being written; that error comes after the answer.
    req.on('error', (error) => {
      if (!answered) {
        reject(error)
      }
    })
    req.end(request.body)
  })
}

export async function startHarness(): Promise<Harness> {
  const binary = rustCombinerBinary()
  // The TS services log every request at info and the errors the rows provoke at error. A passing
  // run prints nothing at fatal, bunyan's highest level. Set before the first `rootLogger` call,
  // which creates the logger, and over the trace level that dotenv reads from apps/combiner/.env.
  process.env.LOG_LEVEL = 'fatal'

  const started: Started = { servers: [], databases: [], rustCombinerOutput: [] }
  try {
    const tsSigners = await startSigners(started)
    const rustSigners = await startSigners(started)

    const tsConfig: typeof config = JSON.parse(JSON.stringify(config))
    tsConfig.phoneNumberPrivacy.odisServices.signers = JSON.stringify(
      tsSigners.map((url) => ({ url })),
    )
    const tsApp = startCombiner(tsConfig, viemClient) as Express
    // A deployed TS combiner runs with NODE_ENV=production, under which Express's error pages
    // carry the status text only, not the stack trace they show under Jest's NODE_ENV=test.
    // Express then also writes each error's stack to console.error; the rows assert those lines.
    tsApp.set('env', 'production')
    const tsUrl = await serve(started, http.createServer(tsApp))
    const nodeUrl = await serve(started, fullNode())
    const rustUrl = await startRustCombiner(started, binary, rustSigners, nodeUrl)

    return {
      send: async (request) => {
        const ts = await sendRaw(tsUrl, request)
        const rust = await sendRaw(rustUrl, request)
        return { ts, rust }
      },
      rustCombinerLog: () => started.rustCombinerOutput.join(''),
      stop: () => stopAll(started),
    }
  } catch (error) {
    await stopAll(started)
    throw error
  }
}
