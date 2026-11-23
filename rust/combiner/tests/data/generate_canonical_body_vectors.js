// Writes canonical_body_vectors.json: what JavaScript's JSON.stringify(JSON.parse(body))
// answers for the bodies the Rust combiner's body writer is tested with, and which of them
// the combiner refuses because it cannot write them as JavaScript does.
//
// Each vector is { description, bodyHex, js, rustRefuses, reason }. `js` is null when
// JSON.parse throws. Node replaces invalid UTF-8 with U+FFFD, as body-parser does.
// `reason` names the BodyError variant the combiner answers with when it refuses.
//
// Run from the repository root:
//   node rust/combiner/tests/data/generate_canonical_body_vectors.js
const fs = require('fs')
const path = require('path')

const ACCOUNT = '0x1be31a94361a391bbafb2a4ccd704f57dc04d4bb'
const BLINDED = 'Zm9vYmFyYmF6'
const SESSION = '3e2d8c1a-0b5f-4f7e-9a62-1f0c5e7d9b44'

const sign = {
  account: ACCOUNT,
  blindedQueryPhoneNumber: BLINDED,
  authenticationMethod: 'wallet_key',
  sessionID: SESSION,
}
const quota = { account: ACCOUNT, authenticationMethod: 'wallet_key', sessionID: SESSION }

const vectors = []
const add = (description, body, rustRefuses = false, reason = null) => {
  const bytes = Buffer.isBuffer(body) ? body : Buffer.from(body, 'utf8')
  let js = null
  try {
    js = JSON.stringify(JSON.parse(bytes.toString('utf8')))
  } catch {}
  vectors.push({ description, bodyHex: bytes.toString('hex'), js, rustRefuses, reason })
}
const refuse = (description, body, reason) => add(description, body, true, reason)

// The SDK writes `version` as undefined, so JSON.stringify leaves it out.
for (const [name, obj] of [
  ['sign', sign],
  ['quota', quota],
]) {
  add(`SDK ${name} body, compact`, JSON.stringify(obj))
  add(`SDK ${name} body, pretty-printed`, JSON.stringify(obj, null, 2))
  add(`SDK ${name} body, CRLF whitespace`, JSON.stringify(obj, null, 2).replace(/\n/g, '\r\n'))
}

const withExtra = (extra) => `{"account":"${ACCOUNT}","extra":${extra}}`
add('emoji', withExtra('"a\u{1F600}b"'))
add('CJK', withExtra('"中文"'))
add('escaped newline and tab', withExtra('"a\\nb\\tc"'))
add('raw escapes \\u0001 and \\u001f', withExtra('"\\u0001\\u001f"'))
add('escaped quote and backslash', withExtra('"\\"\\\\"'))
add('escaped slash', withExtra('"a\\/b"'))
add('U+2028 and U+2029', withExtra('"\u2028\u2029"'))
add('U+007F', withExtra('"\u007f"'))
add('escaped \\b \\f \\r and \\u000b', withExtra('"\\b\\f\\r\\u000b"'))
add('upper case hex in \\u escape', withExtra('"\\u00E9\\u00AB"'))
add('empty key', '{"":1,"a":2}')
add('__proto__ key', '{"__proto__":{"x":1},"a":2}')
add('nested objects and arrays', withExtra('{"b":[1,{"c":[]},[[]],{}],"a":{"z":1,"y":2}}'))
add('true, false and null', '{"a":true,"b":false,"c":null,"d":[true,false,null]}')
add('largest safe integer', '{"n":9007199254740991}')
add('smallest safe integer', '{"n":-9007199254740991}')
add('zero and small integers', '{"a":0,"b":-1,"c":10}')
add('127 levels of nesting', '{"a":' + '['.repeat(126) + ']'.repeat(126) + '}')
add('whitespace before and after', ' \t\r\n{"a":1} \t\r\n')

refuse('1.0', '{"n":1.0}', 'UnsafeNumber')
refuse('1e2', '{"n":1e2}', 'UnsafeNumber')
refuse('1E2', '{"n":1E2}', 'UnsafeNumber')
refuse('-0', '{"n":-0}', 'UnsafeNumber')
refuse('1.5', '{"n":1.5}', 'UnsafeNumber')
// serde_json refuses a number beyond f64 itself, so the reason is its own error.
refuse('1e400', '{"n":1e400}', 'Json')
refuse('2^53', '{"n":9007199254740992}', 'UnsafeNumber')
refuse('-(2^53)', '{"n":-9007199254740992}', 'UnsafeNumber')
refuse('20-digit integer', '{"n":12345678901234567890}', 'UnsafeNumber')
refuse('duplicate key', '{"a":1,"b":2,"a":3}', 'DuplicateKey')
refuse('duplicate key in a nested object', '{"o":{"a":1,"a":2}}', 'DuplicateKey')
refuse('integer-like key "1"', '{"b":1,"1":2}', 'IntegerKey')
refuse('integer-like key "0"', '{"b":1,"0":2}', 'IntegerKey')
refuse('integer-like key in a nested object', '{"o":{"7":2}}', 'IntegerKey')
refuse('lone surrogate escape', '{"sessionID":"\\ud83d"}', 'Json')
refuse(
  'UTF-8 byte order mark',
  Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), Buffer.from('{"a":1}')]),
  'Json',
)
refuse(
  'invalid UTF-8 byte inside a string',
  Buffer.concat([Buffer.from('{"a":"x'), Buffer.from([0xff]), Buffer.from('y"}')]),
  'Json',
)
refuse('128 levels of nesting', '{"a":' + '['.repeat(127) + ']'.repeat(127) + '}', 'Json')
refuse('top-level array', '[1,2]', 'NotAnObject')
refuse('top-level string', '"abc"', 'NotAnObject')
refuse('malformed JSON', '{"a":', 'Json')
refuse('empty body', '', 'Json')

const out = path.join(__dirname, 'canonical_body_vectors.json')
fs.writeFileSync(out, JSON.stringify(vectors, null, 2) + '\n')
console.log(`wrote ${vectors.length} vectors to ${out}`)
