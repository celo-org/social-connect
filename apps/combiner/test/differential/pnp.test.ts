// Sends the same raw requests to the TS combiner and the Rust combiner and compares the answers.
// A row marked `same` must get equal answers from both. A row that names two answers pins a
// difference listed in rust/combiner/CHANGELOG.md, so neither side can drift from it unnoticed.
//
// Run with `just test-differential`, which builds the Rust combiner first.
import {
  ErrorMessage,
  getMessageDigest,
  signWithRawKey,
  TestUtils,
  WarningMessage,
} from '@celo/phone-number-privacy-common'
import { serializeSignature, signMessage } from '@celo/utils/lib/signatureUtils'
import threshold_bls from 'blind-threshold-bls'
import { OutgoingHttpHeaders } from 'http'
import { privateKeyToAccount } from 'viem/accounts'
import { gzipSync } from 'zlib'
import {
  Answer,
  Harness,
  RawRequest,
  startHarness,
  TestUser,
  testUser,
  USER_WITHOUT_DEK,
  USER_WITHOUT_QUOTA,
} from './harness'

const { DEK_PRIVATE_KEY } = TestUtils.Values

jest.setTimeout(30_000)

// The blinded message and combined signatures of the TS integration test
// (apps/combiner/test/integration/pnp.test.ts).
const userSeed = new Uint8Array(32)
for (let i = 0; i < userSeed.length - 1; i++) {
  userSeed[i] = i
}
const BLINDED_QUERY = Buffer.from(
  threshold_bls.blind(Buffer.from('test message', 'utf8'), userSeed).message,
).toString('base64')
const SIGNATURE_V1 = 'xgFMQtcgAMHJAEX/m9B4VFopYtxqPFSw0024sWzRYvQDvnmFqhXOPdnRDfa8WCEA'
const SIGNATURE_V2 = 'wUuFV8yFBXGyEzKbyWjBChG6dER264nwjOsqErd/UZieVKE0oDMZcMDG+qObu4QB'

const SESSION_ID = 'differential-session'

type Fields = Record<string, unknown>

function signFields(user: TestUser, extra: Fields = {}): Fields {
  return {
    account: user.account,
    blindedQueryPhoneNumber: BLINDED_QUERY,
    authenticationMethod: 'wallet_key',
    sessionID: SESSION_ID,
    ...extra,
  }
}

function quotaFields(user: TestUser, extra: Fields = {}): Fields {
  return {
    account: user.account,
    authenticationMethod: 'wallet_key',
    sessionID: SESSION_ID,
    ...extra,
  }
}

/** The compact sign body of `user` with `members` (raw JSON) appended. */
function signBodyWith(user: TestUser, members: string): string {
  return JSON.stringify(signFields(user)).slice(0, -1) + ',' + members + '}'
}

/**
 * The text the TS combiner checks the signature over and forwards: the body read as body-parser
 * reads it (UTF-8, a leading byte order mark dropped), then written by JSON.stringify.
 */
function canonical(body: Buffer): string {
  return JSON.stringify(JSON.parse(body.toString('utf8').replace(/^\uFEFF/, '')))
}

/** A wallet-key signature as `@celo/utils` writes it (v, r, s). */
function walletKeySignature(user: TestUser, message: string): string {
  return serializeSignature(signMessage(message, user.privateKey, user.account))
}

/** A DEK signature in each form elliptic reads. The SDK sends `der`. */
function dekSignatures(message: string, privateKey: string = DEK_PRIVATE_KEY) {
  // elliptic is only imported inline (see eslint.config.js).
  const EC = require('elliptic').ec
  const signature = new EC('secp256k1')
    .keyFromPrivate(Buffer.from(privateKey.replace(/^0x/, ''), 'hex'))
    .sign(getMessageDigest(message))
  return {
    der: JSON.stringify(signature.toDER()),
    hex: JSON.stringify(signature.toDER('hex')),
    rs: JSON.stringify({ r: signature.r.toString(16), s: signature.s.toString(16) }),
  }
}

function post(path: string, body: string | Buffer, headers: OutgoingHttpHeaders): RawRequest {
  const sent: OutgoingHttpHeaders = {}
  for (const [name, value] of Object.entries(headers)) {
    if (value !== undefined) {
      sent[name] = value
    }
  }
  return {
    method: 'POST',
    path,
    headers: sent,
    body: Buffer.isBuffer(body) ? body : Buffer.from(body, 'utf8'),
  }
}

/**
 * POSTs `body` as JSON, signed over its canonical text: with the DEK when it names
 * `encryption_key`, else with the user's wallet key. `headers` override, and `undefined` removes.
 */
function signed(
  path: string,
  user: TestUser,
  body: Fields | string | Buffer,
  headers: OutgoingHttpHeaders = {},
): RawRequest {
  const bytes = Buffer.isBuffer(body)
    ? body
    : Buffer.from(typeof body === 'string' ? body : JSON.stringify(body), 'utf8')
  const message = canonical(bytes)
  const authorization =
    JSON.parse(message).authenticationMethod === 'encryption_key'
      ? dekSignatures(message).der
      : walletKeySignature(user, message)
  return post(path, bytes, { 'content-type': 'application/json', authorization, ...headers })
}

/** A request as the SDK sends it (`packages/identity/src/odis/query.ts`) through cross-fetch. */
async function sdkRequest(path: string, user: TestUser, fields: Fields): Promise<RawRequest> {
  const body = JSON.stringify(fields)
  const authorization =
    fields.authenticationMethod === 'encryption_key'
      ? signWithRawKey(body, DEK_PRIVATE_KEY)
      : await privateKeyToAccount(user.privateKey).signMessage({ message: body })
  return post(path, body, {
    accept: 'application/json',
    'content-type': 'application/json',
    // The SDK sets the header to `undefined` without a key version, and fetch sends it as text.
    'odis-key-version': 'undefined',
    authorization,
    'user-agent': 'node-fetch/1.0 (+https://github.com/bitinn/node-fetch)',
    'accept-encoding': 'gzip,deflate',
  })
}

function request(method: string, path: string): RawRequest {
  return { method, path, headers: {}, body: Buffer.alloc(0) }
}

const CORS = {
  'access-control-allow-origin': '*',
  'access-control-allow-methods': 'GET, POST, OPTIONS',
  'access-control-allow-headers':
    'Origin, X-Requested-With, Content-Type, Accept, Authorization, odis-key-version',
}
const JSON_WITH_CORS = { ...CORS, 'content-type': 'application/json' }

function signedAnswer(signature: string): Answer {
  return {
    status: 200,
    body: JSON.stringify({
      success: true,
      version: '<version>',
      signature,
      performedQueryCount: 1,
      totalQuota: 1000,
      warnings: [],
    }),
    headers: JSON_WITH_CORS,
  }
}

const QUOTA_ANSWER: Answer = {
  status: 200,
  body: JSON.stringify({
    success: true,
    version: '<version>',
    performedQueryCount: 0,
    totalQuota: 1000,
    warnings: [],
  }),
  headers: JSON_WITH_CORS,
}

function failure(status: number, error: string): Answer {
  return {
    status,
    body: JSON.stringify({ success: false, version: '<version>', error }),
    headers: JSON_WITH_CORS,
  }
}

/** Express's error page (finalhandler) under NODE_ENV=production. */
function expressError(status: number, message: string, headers: Record<string, string>): Answer {
  return {
    status,
    body:
      '<!DOCTYPE html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n<title>Error</title>\n' +
      `</head>\n<body>\n<pre>${message}</pre>\n</body>\n</html>\n`,
    headers: { ...headers, 'content-type': 'text/html' },
  }
}

const RUST_EMPTY_404: Answer = { status: 404, body: '', headers: CORS }
const INVALID_INPUT = failure(400, WarningMessage.INVALID_INPUT)
const UNAUTHENTICATED = failure(401, WarningMessage.UNAUTHENTICATED_USER)
const INVALID_KEY_VERSION = failure(400, WarningMessage.INVALID_KEY_VERSION_REQUEST)

type Row = {
  name: string
  /** Builds the request for an account used by no other row, so rows cannot share quota. */
  request: (user: TestUser) => RawRequest | Promise<RawRequest>
  /**
   * The first line of each error stack the TS combiner's Express writes to console.error while
   * answering, in order. Every row asserts this list, empty unless given.
   */
  expressErrors?: string[]
} & (
  | {
      expect: 'same'
      /**
       * The status and error string both answer (no error for a success), so a row cannot pass
       * by both sides failing alike or refusing for the same wrong reason.
       */
      status: number
      error?: string
    }
  | { expect: { ts: Partial<Answer>; rust: Partial<Answer> } }
)

/** A body the Rust combiner refuses with 400 WARN_01 and the TS combiner signs. */
function tsSignsRustRefuses(name: string, body: (user: TestUser) => string | Buffer): Row {
  return {
    name,
    request: (user) => signed('/sign', user, body(user)),
    expect: { ts: signedAnswer(SIGNATURE_V1), rust: INVALID_INPUT },
  }
}

/** A body with an extra field that both combiners accept and sign. */
function extraField(name: string, members: string): Row {
  return {
    name: `/sign with an extra field: ${name}`,
    request: (user) => signed('/sign', user, signBodyWith(user, members)),
    expect: 'same',
    status: 200,
  }
}

/** The extra field that makes the canonical sign body exactly `length` characters long. */
function paddedTo(user: TestUser, length: number): Fields {
  const empty = JSON.stringify(signFields(user, { padding: '' })).length
  return signFields(user, { padding: 'x'.repeat(length - empty) })
}

const rows: Row[] = [
  // Both endpoints, both authentication methods, each key version.
  {
    name: '/sign with wallet key',
    request: (user) => signed('/sign', user, signFields(user)),
    expect: 'same',
    status: 200,
  },
  ...[1, 2, 3].map(
    (version): Row => ({
      name: `/sign with key version ${version}`,
      request: (user) =>
        signed('/sign', user, signFields(user), { 'odis-key-version': String(version) }),
      expect: 'same',
      status: 200,
    }),
  ),
  {
    name: '/sign with DEK',
    request: (user) =>
      signed('/sign', user, signFields(user, { authenticationMethod: 'encryption_key' })),
    expect: 'same',
    status: 200,
  },
  {
    name: '/quotaStatus with wallet key',
    request: (user) => signed('/quotaStatus', user, quotaFields(user)),
    expect: 'same',
    status: 200,
  },
  {
    name: '/quotaStatus with DEK',
    request: (user) =>
      signed('/quotaStatus', user, quotaFields(user, { authenticationMethod: 'encryption_key' })),
    expect: 'same',
    status: 200,
  },

  // The SDK's bytes and headers (packages/identity/src/odis/identifier.ts and quota.ts).
  {
    name: '/sign from the SDK with wallet key',
    request: (user) =>
      sdkRequest('/sign', user, {
        account: user.account,
        blindedQueryPhoneNumber: BLINDED_QUERY,
        version: undefined,
        authenticationMethod: 'wallet_key',
        sessionID: SESSION_ID,
      }),
    expect: 'same',
    status: 200,
  },
  {
    name: '/sign from the SDK with DEK and a client version',
    request: (user) =>
      sdkRequest('/sign', user, {
        account: user.account,
        blindedQueryPhoneNumber: BLINDED_QUERY,
        version: '5.1.2',
        authenticationMethod: 'encryption_key',
        sessionID: SESSION_ID,
      }),
    expect: 'same',
    status: 200,
  },
  {
    name: '/quotaStatus from the SDK with wallet key',
    request: (user) =>
      sdkRequest('/quotaStatus', user, {
        account: user.account,
        version: undefined,
        authenticationMethod: 'wallet_key',
        sessionID: SESSION_ID,
      }),
    expect: 'same',
    status: 200,
  },
  {
    name: '/quotaStatus from the SDK with DEK',
    request: (user) =>
      sdkRequest('/quotaStatus', user, {
        account: user.account,
        version: undefined,
        authenticationMethod: 'encryption_key',
        sessionID: SESSION_ID,
      }),
    expect: 'same',
    status: 200,
  },

  // Whitespace: the signature is checked over the compact form.
  {
    name: '/sign signed compact and sent with whitespace, wallet key',
    request: (user) => signed('/sign', user, JSON.stringify(signFields(user), null, 2)),
    expect: 'same',
    status: 200,
  },
  {
    name: '/sign signed compact and sent with whitespace, DEK',
    request: (user) =>
      signed(
        '/sign',
        user,
        JSON.stringify(signFields(user, { authenticationMethod: 'encryption_key' }), null, 2),
      ),
    expect: 'same',
    status: 200,
  },
  {
    name: '/sign signed and sent with whitespace',
    request: (user) => {
      const body = JSON.stringify(signFields(user), null, 2)
      return post('/sign', body, {
        'content-type': 'application/json',
        authorization: walletKeySignature(user, body),
      })
    },
    expect: 'same',
    status: 401,
    error: WarningMessage.UNAUTHENTICATED_USER,
  },

  // Quota.
  {
    name: '/sign out of quota',
    request: () => signed('/sign', USER_WITHOUT_QUOTA, signFields(USER_WITHOUT_QUOTA)),
    expect: 'same',
    status: 403,
    error: WarningMessage.EXCEEDED_QUOTA,
  },
  {
    name: '/quotaStatus without quota',
    request: () => signed('/quotaStatus', USER_WITHOUT_QUOTA, quotaFields(USER_WITHOUT_QUOTA)),
    expect: 'same',
    status: 200,
  },

  // Extra fields that both write back alike.
  extraField('emoji', '"extra":"a\u{1F600}b"'),
  extraField('CJK', '"extra":"中文"'),
  extraField('escaped newline and tab', '"extra":"a\\nb\\tc"'),
  extraField('escapes \\u0001 and \\u001f', '"extra":"\\u0001\\u001f"'),
  extraField('escaped quote and backslash', '"extra":"\\"\\\\"'),
  extraField('escaped slash', '"extra":"a\\/b"'),
  extraField('U+2028 and U+2029', '"extra":"\u2028\u2029"'),
  extraField('U+007F', '"extra":"\u007f"'),
  extraField('escapes \\b \\f \\r and \\u000b', '"extra":"\\b\\f\\r\\u000b"'),
  extraField('upper-case hex escapes', '"extra":"\\u00E9\\u00AB"'),
  extraField('empty key', '"":1'),
  extraField('__proto__ key', '"__proto__":{"x":1}'),
  extraField('nested objects and arrays', '"extra":{"b":[1,{"c":[]},[[]],{}],"a":{"z":1,"y":2}}'),
  extraField('true, false and null', '"extra":[true,false,null]'),
  extraField(
    'largest and smallest safe integers',
    '"max":9007199254740991,"min":-9007199254740991',
  ),
  extraField('127 levels of nesting', `"extra":${'['.repeat(126)}${']'.repeat(126)}`),
  {
    name: '/sign with a canonical body of 16,000 characters',
    request: (user) => signed('/sign', user, paddedTo(user, 16_000)),
    expect: 'same',
    status: 200,
  },

  // Key versions.
  {
    name: '/sign with an unsupported key version and a bad signature',
    request: (user) =>
      signed('/sign', user, signFields(user), {
        'odis-key-version': '4',
        authorization: walletKeySignature(user, 'something else'),
      }),
    expect: 'same',
    status: 400,
    error: WarningMessage.INVALID_KEY_VERSION_REQUEST,
  },
  {
    name: '/quotaStatus with an unsupported key version and a bad signature',
    request: (user) =>
      signed('/quotaStatus', user, quotaFields(user), {
        'odis-key-version': '4',
        authorization: walletKeySignature(user, 'something else'),
      }),
    expect: 'same',
    status: 401,
    error: WarningMessage.UNAUTHENTICATED_USER,
  },
  {
    // Both read it as a version that is not configured; on /quotaStatus they differ (below).
    name: '/sign with key version 4294967296',
    request: (user) =>
      signed('/sign', user, signFields(user), { 'odis-key-version': '4294967296' }),
    expect: 'same',
    status: 400,
    error: WarningMessage.INVALID_KEY_VERSION_REQUEST,
  },
  {
    name: '/sign with key version "invalid"',
    request: (user) => signed('/sign', user, signFields(user), { 'odis-key-version': 'invalid' }),
    expect: 'same',
    status: 200,
  },
  {
    name: '/sign with key version " 2 "',
    request: (user) => signed('/sign', user, signFields(user), { 'odis-key-version': ' 2 ' }),
    expect: 'same',
    status: 200,
  },
  {
    // TS reads 1.0 as version 1 and Rust as absent, which is the current version 1 too.
    name: '/sign with key version 1.0',
    request: (user) => signed('/sign', user, signFields(user), { 'odis-key-version': '1.0' }),
    expect: 'same',
    status: 200,
  },
  {
    // Node joins the values to `2, 3`, which TS reads as NaN; both use the current version.
    name: '/sign with the key version header sent twice',
    request: (user) => signed('/sign', user, signFields(user), { 'odis-key-version': ['2', '3'] }),
    expect: 'same',
    status: 200,
  },

  // Accounts and authentication.
  {
    name: '/sign with an upper-case account, wallet key',
    request: (user) => {
      const upper = { ...user, account: `0x${user.account.slice(2).toUpperCase()}` as const }
      return signed('/sign', upper, signFields(upper))
    },
    expect: 'same',
    status: 401,
    error: WarningMessage.UNAUTHENTICATED_USER,
  },
  {
    name: '/sign with an upper-case account, DEK',
    request: (user) => {
      const upper = { ...user, account: `0x${user.account.slice(2).toUpperCase()}` as const }
      return signed('/sign', upper, signFields(upper, { authenticationMethod: 'encryption_key' }))
    },
    expect: 'same',
    status: 401,
    error: WarningMessage.UNAUTHENTICATED_USER,
  },
  {
    name: '/sign with an account without 0x',
    request: (user) => signed('/sign', user, signFields(user, { account: user.account.slice(2) })),
    expect: 'same',
    status: 401,
    error: WarningMessage.UNAUTHENTICATED_USER,
  },
  {
    name: '/sign with an account with 0X',
    request: (user) =>
      signed('/sign', user, signFields(user, { account: `0X${user.account.slice(2)}` })),
    expect: 'same',
    status: 401,
    error: WarningMessage.UNAUTHENTICATED_USER,
  },
  {
    name: '/sign without an Authorization header',
    request: (user) => signed('/sign', user, signFields(user), { authorization: undefined }),
    expect: 'same',
    status: 401,
    error: WarningMessage.UNAUTHENTICATED_USER,
  },
  {
    name: '/sign signed by another wallet key',
    request: (user) => signed('/sign', testUser(`${user.account} other`), signFields(user)),
    expect: 'same',
    status: 401,
    error: WarningMessage.UNAUTHENTICATED_USER,
  },
  {
    name: '/sign with a wallet-key signature without 0x',
    request: (user) => {
      const body = JSON.stringify(signFields(user))
      return post('/sign', body, {
        'content-type': 'application/json',
        authorization: walletKeySignature(user, body).slice(2),
      })
    },
    expect: 'same',
    status: 401,
    error: WarningMessage.UNAUTHENTICATED_USER,
  },
  {
    name: '/sign with DEK signed by another key',
    request: (user) => {
      const body = JSON.stringify(signFields(user, { authenticationMethod: 'encryption_key' }))
      const otherKey = '0x00000000000000000000000000000000000000000000000000000000ddddbbbb'
      return post('/sign', body, {
        'content-type': 'application/json',
        authorization: dekSignatures(body, otherKey).der,
      })
    },
    expect: 'same',
    status: 401,
    error: WarningMessage.UNAUTHENTICATED_USER,
  },
  {
    name: '/sign with DEK signed by the wallet key falls back to it',
    request: (user) => {
      const body = JSON.stringify(signFields(user, { authenticationMethod: 'encryption_key' }))
      return post('/sign', body, {
        'content-type': 'application/json',
        authorization: walletKeySignature(user, body),
      })
    },
    expect: 'same',
    status: 200,
  },
  {
    name: '/sign with DEK for an account without a DEK, signed by the wallet key',
    request: () => {
      const body = JSON.stringify(
        signFields(USER_WITHOUT_DEK, { authenticationMethod: 'encryption_key' }),
      )
      return post('/sign', body, {
        'content-type': 'application/json',
        authorization: walletKeySignature(USER_WITHOUT_DEK, body),
      })
    },
    expect: 'same',
    status: 200,
  },

  // Bodies both refuse.
  {
    name: '/sign without an account',
    request: (user) => {
      const { account: _, ...fields } = signFields(user)
      return signed('/sign', user, fields)
    },
    expect: 'same',
    status: 400,
    error: WarningMessage.INVALID_INPUT,
  },
  {
    name: '/sign with an account of 39 hex digits',
    request: (user) =>
      signed('/sign', user, signFields(user, { account: user.account.slice(0, -1) })),
    expect: 'same',
    status: 400,
    error: WarningMessage.INVALID_INPUT,
  },
  {
    name: '/sign with an account as an array of 20 numbers',
    request: (user) =>
      signed(
        '/sign',
        user,
        signFields(user, { account: [...Buffer.from(user.account.slice(2), 'hex')] }),
      ),
    expect: 'same',
    status: 400,
    error: WarningMessage.INVALID_INPUT,
  },
  {
    name: '/sign without a blinded query',
    request: (user) => {
      const { blindedQueryPhoneNumber: _, ...fields } = signFields(user)
      return signed('/sign', user, fields)
    },
    expect: 'same',
    status: 400,
    error: WarningMessage.INVALID_INPUT,
  },
  {
    name: '/sign with a blinded query of 63 characters',
    request: (user) =>
      signed('/sign', user, signFields(user, { blindedQueryPhoneNumber: BLINDED_QUERY.slice(1) })),
    expect: 'same',
    status: 400,
    error: WarningMessage.INVALID_INPUT,
  },
  {
    name: '/sign with a blinded query that is not base64',
    request: (user) =>
      signed(
        '/sign',
        user,
        signFields(user, { blindedQueryPhoneNumber: '!' + BLINDED_QUERY.slice(1) }),
      ),
    expect: 'same',
    status: 400,
    error: WarningMessage.INVALID_INPUT,
  },
  {
    name: '/sign with a numeric sessionID',
    request: (user) => signed('/sign', user, signFields(user, { sessionID: 7 })),
    expect: 'same',
    status: 400,
    error: WarningMessage.INVALID_INPUT,
  },
  ...['authenticationMethod', 'sessionID', 'version'].map(
    (field): Row => ({
      name: `/sign with ${field} null`,
      request: (user) => signed('/sign', user, signFields(user, { [field]: null })),
      expect: 'same',
      status: 400,
      error: WarningMessage.INVALID_INPUT,
    }),
  ),
  {
    name: '/quotaStatus without an account',
    request: (user) => {
      const { account: _, ...fields } = quotaFields(user)
      return signed('/quotaStatus', user, fields)
    },
    expect: 'same',
    status: 400,
    error: WarningMessage.INVALID_INPUT,
  },
  {
    name: '/quotaStatus with authenticationMethod null',
    request: (user) =>
      signed('/quotaStatus', user, quotaFields(user, { authenticationMethod: null })),
    expect: 'same',
    status: 400,
    error: WarningMessage.INVALID_INPUT,
  },
  {
    name: '/sign with a top-level array',
    request: (user) => signed('/sign', user, JSON.stringify([signFields(user)])),
    expect: 'same',
    status: 400,
    error: WarningMessage.INVALID_INPUT,
  },
  {
    name: '/sign with an empty body',
    request: () => post('/sign', '', { 'content-type': 'application/json' }),
    expect: 'same',
    status: 400,
    error: WarningMessage.INVALID_INPUT,
  },
  {
    name: '/sign with a canonical body of 16,001 characters',
    request: (user) => signed('/sign', user, paddedTo(user, 16_001)),
    expect: 'same',
    status: 400,
    error: WarningMessage.INVALID_INPUT,
  },
  {
    name: '/sign with Content-Type text/plain',
    request: (user) => signed('/sign', user, signFields(user), { 'content-type': 'text/plain' }),
    expect: 'same',
    status: 400,
    error: WarningMessage.INVALID_INPUT,
  },
  {
    name: '/sign without a Content-Type',
    request: (user) => signed('/sign', user, signFields(user), { 'content-type': undefined }),
    expect: 'same',
    status: 400,
    error: WarningMessage.INVALID_INPUT,
  },

  // Content types that real HTTP libraries send.
  {
    name: '/sign with Content-Type application/json;charset=UTF-8',
    request: (user) =>
      signed('/sign', user, signFields(user), {
        'content-type': 'application/json;charset=UTF-8',
      }),
    expect: 'same',
    status: 200,
  },
  {
    name: '/sign with Content-Type application/json; charset="utf-8"',
    request: (user) =>
      signed('/sign', user, signFields(user), {
        'content-type': 'application/json; charset="utf-8"',
      }),
    expect: 'same',
    status: 200,
  },

  // Content types that type-is parses, or not; Express skips the body of one it cannot parse.
  ...[
    'application/json; charset=""',
    'application/json; charset=latin1; charset=utf-8',
    'application/json ; foo = "a;b"',
  ].map(
    (contentType): Row => ({
      name: `/sign with Content-Type ${contentType}`,
      request: (user) => signed('/sign', user, signFields(user), { 'content-type': contentType }),
      expect: 'same',
      status: 200,
    }),
  ),
  ...[
    'application/json;',
    'application/json; foo',
    'application/json; charset=utf-8; foo',
    'application/json; charset=',
  ].map(
    (contentType): Row => ({
      name: `/sign with Content-Type ${contentType}`,
      request: (user) => signed('/sign', user, signFields(user), { 'content-type': contentType }),
      expect: 'same',
      status: 400,
      error: WarningMessage.INVALID_INPUT,
    }),
  ),

  // Content encodings body-parser reads as none.
  ...['identity', 'IDENTITY', ''].map(
    (encoding): Row => ({
      name: `/sign with Content-Encoding "${encoding}"`,
      request: (user) => signed('/sign', user, signFields(user), { 'content-encoding': encoding }),
      expect: 'same',
      status: 200,
    }),
  ),

  // Routing and transport ("Routes and transport" in the changelog).
  ...['/sign', '/quotaStatus'].map(
    (path): Row => ({
      // Express answers OPTIONS itself for a path it routes, with the methods as the body.
      name: `OPTIONS ${path}`,
      request: () => request('OPTIONS', path),
      expect: {
        ts: {
          status: 200,
          body: 'POST',
          headers: { ...CORS, allow: 'POST', 'content-type': 'text/html' },
        },
        rust: { status: 200, body: '', headers: { ...CORS, allow: 'POST' } },
      },
    }),
  ),
  {
    name: '/SIGN',
    request: (user) => signed('/SIGN', user, signFields(user)),
    expect: { ts: signedAnswer(SIGNATURE_V1), rust: RUST_EMPTY_404 },
  },
  {
    name: '/quotastatus',
    request: (user) => signed('/quotastatus', user, quotaFields(user)),
    expect: { ts: QUOTA_ANSWER, rust: RUST_EMPTY_404 },
  },
  {
    name: '/sign/',
    request: (user) => signed('/sign/', user, signFields(user)),
    expect: { ts: signedAnswer(SIGNATURE_V1), rust: RUST_EMPTY_404 },
  },
  {
    name: 'GET /sign',
    request: () => request('GET', '/sign'),
    expect: {
      ts: expressError(404, 'Cannot GET /sign', CORS),
      rust: { status: 405, body: '', headers: { ...CORS, allow: 'POST' } },
    },
  },
  {
    name: 'GET on an unknown path',
    request: () => request('GET', '/unknown'),
    expect: { ts: expressError(404, 'Cannot GET /unknown', CORS), rust: RUST_EMPTY_404 },
  },
  {
    name: 'OPTIONS on an unknown path',
    request: () => request('OPTIONS', '/unknown'),
    expect: {
      ts: expressError(404, 'Cannot OPTIONS /unknown', CORS),
      rust: { status: 200, body: '', headers: CORS },
    },
  },
  {
    name: '/sign with Content-Encoding gzip',
    request: (user) => {
      const body = JSON.stringify(signFields(user))
      return post('/sign', gzipSync(body), {
        'content-type': 'application/json',
        'content-encoding': 'gzip',
        authorization: walletKeySignature(user, body),
      })
    },
    expect: { ts: signedAnswer(SIGNATURE_V1), rust: INVALID_INPUT },
  },
  {
    name: '/sign with Content-Encoding gzip and a plain body',
    request: (user) => signed('/sign', user, signFields(user), { 'content-encoding': 'gzip' }),
    expressErrors: ['BadRequestError: Bad Request'],
    expect: { ts: expressError(400, 'Bad Request', {}), rust: INVALID_INPUT },
  },
  {
    name: '/sign with Content-Encoding br',
    request: (user) => signed('/sign', user, signFields(user), { 'content-encoding': 'br' }),
    expressErrors: ['UnsupportedMediaTypeError: unsupported content encoding "br"'],
    expect: { ts: expressError(415, 'Unsupported Media Type', {}), rust: INVALID_INPUT },
  },
  {
    // Node joins the values to `identity, identity`, which body-parser does not know.
    name: '/sign with Content-Encoding identity sent twice',
    request: (user) =>
      signed('/sign', user, signFields(user), { 'content-encoding': ['identity', 'identity'] }),
    expressErrors: ['UnsupportedMediaTypeError: unsupported content encoding "identity, identity"'],
    expect: { ts: expressError(415, 'Unsupported Media Type', {}), rust: INVALID_INPUT },
  },
  {
    name: '/sign with malformed JSON',
    request: () => post('/sign', '{"account":', { 'content-type': 'application/json' }),
    expressErrors: ['SyntaxError: Unexpected end of JSON input'],
    expect: { ts: expressError(400, 'Bad Request', {}), rust: INVALID_INPUT },
  },
  {
    name: '/sign with charset utf-16',
    request: (user) => {
      const body = JSON.stringify(signFields(user))
      return post(
        '/sign',
        Buffer.concat([Buffer.from([0xff, 0xfe]), Buffer.from(body, 'utf16le')]),
        {
          'content-type': 'application/json; charset=utf-16',
          authorization: walletKeySignature(user, body),
        },
      )
    },
    expect: { ts: signedAnswer(SIGNATURE_V1), rust: INVALID_INPUT },
  },
  {
    name: '/sign with charset latin1',
    request: (user) =>
      signed('/sign', user, signFields(user), {
        'content-type': 'application/json; charset=latin1',
      }),
    expressErrors: ['UnsupportedMediaTypeError: unsupported charset "LATIN1"'],
    expect: { ts: expressError(415, 'Unsupported Media Type', {}), rust: INVALID_INPUT },
  },
  {
    name: '/sign with a body of 300 KB',
    request: (user) => signed('/sign', user, paddedTo(user, 300 * 1024)),
    expressErrors: ['PayloadTooLargeError: request entity too large'],
    expect: {
      ts: expressError(413, 'Payload Too Large', {}),
      rust: {
        status: 413,
        body: 'length limit exceeded',
        headers: { ...CORS, 'content-type': 'text/plain' },
      },
    },
  },
  {
    // Without a Content-Length the limit is met while the body is read, not before.
    name: '/sign with a chunked body of 300 KB',
    request: (user) =>
      signed('/sign', user, paddedTo(user, 300 * 1024), { 'transfer-encoding': 'chunked' }),
    expressErrors: ['PayloadTooLargeError: request entity too large'],
    expect: {
      ts: expressError(413, 'Payload Too Large', {}),
      rust: {
        status: 413,
        body: 'Failed to buffer the request body: length limit exceeded',
        headers: { ...CORS, 'content-type': 'text/plain' },
      },
    },
  },

  // Request fields ("Request fields" in the changelog).
  {
    name: '/quotaStatus with an unsupported key version',
    request: (user) => signed('/quotaStatus', user, quotaFields(user), { 'odis-key-version': '4' }),
    expect: { ts: failure(500, ErrorMessage.UNKNOWN_ERROR), rust: INVALID_KEY_VERSION },
  },
  {
    name: '/quotaStatus with key version 4294967296',
    request: (user) =>
      signed('/quotaStatus', user, quotaFields(user), { 'odis-key-version': '4294967296' }),
    expect: { ts: failure(500, ErrorMessage.UNKNOWN_ERROR), rust: INVALID_KEY_VERSION },
  },
  {
    name: '/sign with key version 2.0',
    request: (user) => signed('/sign', user, signFields(user), { 'odis-key-version': '2.0' }),
    expect: { ts: signedAnswer(SIGNATURE_V2), rust: signedAnswer(SIGNATURE_V1) },
  },
  {
    name: '/sign with a key version of 400 digits',
    request: (user) =>
      signed('/sign', user, signFields(user), { 'odis-key-version': '1'.repeat(400) }),
    expect: { ts: signedAnswer(SIGNATURE_V1), rust: INVALID_KEY_VERSION },
  },
  {
    // TS's is-base64 accepts padding bits that are not zero; the signers then cannot sign.
    name: '/sign with a blinded query in non-canonical base64',
    request: (user) =>
      signed('/sign', user, signFields(user, { blindedQueryPhoneNumber: 'A'.repeat(61) + 'B==' })),
    expect: {
      ts: failure(500, ErrorMessage.NOT_ENOUGH_PARTIAL_SIGNATURES),
      rust: INVALID_INPUT,
    },
  },

  // Bodies the Rust combiner cannot write back as JavaScript does ("Request bodies").
  tsSignsRustRefuses('/sign with 1.0', (user) => signBodyWith(user, '"extra":1.0')),
  tsSignsRustRefuses('/sign with 1e2', (user) => signBodyWith(user, '"extra":1e2')),
  tsSignsRustRefuses('/sign with -0', (user) => signBodyWith(user, '"extra":-0')),
  tsSignsRustRefuses('/sign with 1e400', (user) => signBodyWith(user, '"extra":1e400')),
  tsSignsRustRefuses('/sign with a 20-digit integer', (user) =>
    signBodyWith(user, '"extra":12345678901234567890'),
  ),
  tsSignsRustRefuses('/sign with a duplicate key', (user) =>
    signBodyWith(user, '"extra":1,"extra":2'),
  ),
  tsSignsRustRefuses('/sign with an integer-like key', (user) => signBodyWith(user, '"1":"x"')),
  tsSignsRustRefuses('/sign with a lone surrogate escape', (user) =>
    signBodyWith(user, '"extra":"\\ud83d"'),
  ),
  tsSignsRustRefuses('/sign with a byte order mark', (user) =>
    Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), Buffer.from(JSON.stringify(signFields(user)))]),
  ),
  tsSignsRustRefuses('/sign with an invalid UTF-8 byte', (user) =>
    Buffer.concat([
      Buffer.from(signBodyWith(user, '"extra":"x').slice(0, -1)),
      Buffer.from([0xff]),
      Buffer.from('y"}'),
    ]),
  ),
  tsSignsRustRefuses('/sign with 128 levels of nesting', (user) =>
    signBodyWith(user, `"extra":${'['.repeat(127)}${']'.repeat(127)}`),
  ),

  // Signatures the TS services read loosely ("Authentication" in the changelog).
  {
    name: '/sign with a wallet-key signature prefixed 0X',
    request: (user) => {
      const body = JSON.stringify(signFields(user))
      return post('/sign', body, {
        'content-type': 'application/json',
        authorization: '0X' + walletKeySignature(user, body).slice(2),
      })
    },
    expect: { ts: signedAnswer(SIGNATURE_V1), rust: UNAUTHENTICATED },
  },
  {
    name: '/sign with a DEK signature as a hex string',
    request: (user) => {
      const body = JSON.stringify(signFields(user, { authenticationMethod: 'encryption_key' }))
      return post('/sign', body, {
        'content-type': 'application/json',
        authorization: dekSignatures(body).hex,
      })
    },
    expect: { ts: signedAnswer(SIGNATURE_V1), rust: UNAUTHENTICATED },
  },
  {
    name: '/sign with a DEK signature as r and s',
    request: (user) => {
      const body = JSON.stringify(signFields(user, { authenticationMethod: 'encryption_key' }))
      return post('/sign', body, {
        'content-type': 'application/json',
        authorization: dekSignatures(body).rs,
      })
    },
    expect: { ts: signedAnswer(SIGNATURE_V1), rust: UNAUTHENTICATED },
  },
]

/** The `error` of a JSON answer, or undefined. */
function errorOf(answer: Answer): unknown {
  try {
    return JSON.parse(answer.body)?.error
  } catch {
    return undefined
  }
}

/** The fields of `answer` that `expected` names. */
function only(answer: Answer, expected: Partial<Answer>): Partial<Answer> {
  const picked: Partial<Answer> = {}
  for (const key of Object.keys(expected) as Array<keyof Answer>) {
    ;(picked as any)[key] = answer[key]
  }
  return picked
}

/** Sends the row's request to both combiners and checks the answers and Express's error lines. */
async function checkRow(harness: Harness, row: Row): Promise<void> {
  const request = await row.request(testUser(row.name))
  // Express logs synchronously before it answers, so every line is in by the answer.
  const consoleError = jest.spyOn(console, 'error').mockImplementation(() => undefined)
  let answers: { ts: Answer; rust: Answer }
  let logged: string[]
  try {
    answers = await harness.send(request)
  } finally {
    logged = consoleError.mock.calls.map((args) => String(args[0]).split('\n')[0])
    consoleError.mockRestore()
  }
  const { ts, rust } = answers

  expect(logged).toStrictEqual(row.expressErrors ?? [])
  if (row.expect === 'same') {
    expect(rust).toStrictEqual(ts)
    expect({ status: ts.status, error: errorOf(ts) }).toStrictEqual({
      status: row.status,
      error: row.error,
    })
  } else {
    expect(only(ts, row.expect.ts)).toStrictEqual(row.expect.ts)
    expect(only(rust, row.expect.rust)).toStrictEqual(row.expect.rust)
  }
}

describe('the Rust combiner against the TS combiner', () => {
  let harness: Harness
  let rowFailed = false

  beforeAll(async () => {
    harness = await startHarness()
  }, 120_000)

  afterAll(async () => {
    await harness?.stop()
    // Once, after the combiner has stopped, so a failed run shows everything it logged.
    if (rowFailed) {
      console.log(`The Rust combiner's log:\n${harness?.rustCombinerLog()}`)
    }
  })

  it('has a name of its own for every row', () => {
    const names = rows.map((row) => row.name)
    expect(names.filter((name, i) => names.indexOf(name) !== i)).toEqual([])
  })

  it.each(rows)('$name', async (row) => {
    try {
      await checkRow(harness, row)
    } catch (error) {
      rowFailed = true
      throw error
    }
  })
})
