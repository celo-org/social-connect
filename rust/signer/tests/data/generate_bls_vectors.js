// Writes bls_vectors.json: what the TS signer's BLS library answers for a set of
// inputs. The Rust signer's tests assert the same answers, because partial
// signatures from TS and Rust signers must combine.
//
// Run from the repository root, with the TS dependencies installed:
//   node rust/signer/tests/data/generate_bls_vectors.js
const crypto = require('crypto')
const fs = require('fs')
const path = require('path')
const bls = require('blind-threshold-bls')
const { version } = require('blind-threshold-bls/package.json')

const hex = (bytes) => Buffer.from(bytes).toString('hex')
const seed = (label) => crypto.createHash('sha256').update(label).digest()
const sign = (share, blindedMessage) => {
  try {
    return hex(
      bls.partialSignBlindedMessage(Buffer.from(share, 'hex'), Buffer.from(blindedMessage, 'hex')),
    )
  } catch (_) {
    return null
  }
}

// Share 1 of the dev keys for key versions 1 to 3, as in the mock key provider.
const DEV_SHARES = [
  '000000000e7e1a2fad3b54deb2b1b32cf4c7b084842d50bbb5c6143b9d9577d16e050f03',
  '0000000087c722e1338395b942d8332328795a46c718baeb8fef9e5c63111d495469c50e',
  '000000005b2c8089ead28a08233b6b16b2341542453523445950cfbd9bd2f1d09c8eee0c',
]
const blindedMessages = Array.from({ length: 50 }, (_, i) =>
  hex(bls.blind(Buffer.from(`message ${i}`), seed(`blinding ${i}`)).message),
)

const valid = []
for (const blindedMessage of blindedMessages) {
  for (const share of DEV_SHARES) {
    valid.push({ share, blindedMessage, signature: sign(share, blindedMessage) })
  }
}
// Shares with other indexes: all five shares of a fresh 3-of-5 key.
const keys = bls.thresholdKeygen(5, 3, seed('threshold key'))
for (let index = 0; index < keys.numShares(); index++) {
  const share = hex(keys.getShare(index))
  for (const blindedMessage of blindedMessages.slice(0, 2)) {
    valid.push({ share, blindedMessage, signature: sign(share, blindedMessage) })
  }
}
if (valid.some((vector) => vector.signature === null)) {
  throw new Error('a valid input was refused')
}

// Inputs at the edges of what the encodings allow. A point is 48 bytes: the x
// coordinate, little-endian, with two flags in the top bits of the last byte
// (0x80: the larger y, 0x40: the point at infinity). A share is a 4-byte index
// and a 32-byte scalar, both little-endian.
const SHARE = DEV_SHARES[0]
const POINT = blindedMessages[0]
const ORDER = '010000000080110a010000d0fe76aa5901b0375c1e4db46056a52c9a5e65ab12'
const scalarShare = (scalar) => '00000000' + scalar
const edgeInputs = [
  ['point at infinity', SHARE, '00'.repeat(47) + '40'],
  ['point at infinity with the y flag set', SHARE, '00'.repeat(47) + 'c0'],
  ['all zero bytes', SHARE, '00'.repeat(48)],
  [
    'point on the curve but outside the prime-order subgroup',
    SHARE,
    'fb7edf01a0a8b5c112d884a4349bed60a1b88052dd91d4dc25cae3fd6cb7140fa66a20f2058ae57837377a7c1ae4e700',
  ],
  ['x coordinate with no point on the curve', SHARE, '07' + '00'.repeat(47)],
  [
    'x coordinate not reduced modulo the field prime',
    SHARE,
    'a0f23db2b9a2c9c6e6e68edd238b386440c1873821f2078abb103bff22e86641a5a090031b2cbb933548dd24da6e2683',
  ],
  ['point with a trailing byte', SHARE, POINT + '00'],
  ['point one byte short', SHARE, POINT.slice(0, -2)],
  ['empty point', SHARE, ''],
  ['scalar zero', scalarShare('00'.repeat(32)), POINT],
  ['scalar one', scalarShare('01' + '00'.repeat(31)), POINT],
  ['scalar one below the group order', scalarShare('00' + ORDER.slice(2)), POINT],
  ['scalar equal to the group order', scalarShare(ORDER), POINT],
  ['scalar one above the group order', scalarShare('02' + ORDER.slice(2)), POINT],
  ['scalar of all one bits', scalarShare('ff'.repeat(32)), POINT],
  ['share with a trailing byte', SHARE + '00', POINT],
  ['share one byte short', SHARE.slice(0, -2), POINT],
  ['share with the highest index', 'ffffffff' + SHARE.slice(8), POINT],
  ['empty share', '', POINT],
]
const edge = edgeInputs.map(([description, share, blindedMessage]) => ({
  description,
  share,
  blindedMessage,
  signature: sign(share, blindedMessage),
}))

const vectors = {
  source: `@celo/blind-threshold-bls ${version}, partialSignBlindedMessage; a null signature means the library refused the input`,
  valid,
  edge,
}
fs.writeFileSync(path.join(__dirname, 'bls_vectors.json'), JSON.stringify(vectors, null, 2) + '\n')
console.log(`${valid.length} valid vectors, ${edge.length} edge inputs`)
