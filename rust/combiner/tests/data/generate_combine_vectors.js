// Writes combine_vectors.json: what the TS combiner's BLS library answers when it
// verifies partial signatures and combines them. The Rust combiner's tests assert
// the same answers, because it must accept the signatures the TS stack accepts and
// produce the bytes the TS stack produces.
//
// Run from the repository root, with the TS dependencies installed:
//   node rust/combiner/tests/data/generate_combine_vectors.js
const crypto = require('crypto')
const fs = require('fs')
const path = require('path')
const bls = require('blind-threshold-bls')
const { version } = require('blind-threshold-bls/package.json')

const hex = (bytes) => Buffer.from(bytes).toString('hex')
const sha256 = (label) => crypto.createHash('sha256').update(label).digest()

// The dev key versions 1 to 3 from odis-combiner-4.0.5:packages/common/src/test/values.ts
// (PNP_THRESHOLD_DEV_PK_SHARE_{1,2,3}_V{n}, PNP_THRESHOLD_DEV_POLYNOMIAL_V{n},
// PNP_THRESHOLD_DEV_PUBKEY_V{n}).
const DEV_KEYS = [
  {
    keyVersion: 1,
    polynomial:
      '0200000000000000eb569eb8701d831a0a9ffe5df325eed2ac7f5693c758c02aad5804009fcca42ece73f7f560b3e237b60a6b0f5e0d6501516b38b0f9fa20dd944c66edf798cfea73bc232983e0885b04e3632dc125b2ad63c13676723b931d5f686d2c83165d817aaff1f84d0b008ad218eff19db698f343168cf931ba8347640123a2f826f62b66ff084273f494d4647758e9a9f889009d573705824a0e74e1f49ed234462058e53bbb4fef370b55f78da89df070c661782a84239b8c7623d09e34b9f91f7781',
    pubKey:
      '61aeuHAdgxoKn/5d8yXu0qx/VpPHWMAqrVgEAJ/MpC7Oc/f1YLPiN7YKaw9eDWUBUWs4sPn6IN2UTGbt95jP6nO8IymD4IhbBONjLcElsq1jwTZ2cjuTHV9obSyDFl2B',
    shares: [
      '000000000e7e1a2fad3b54deb2b1b32cf4c7b084842d50bbb5c6143b9d9577d16e050f03',
      '01000000e43f10f7778e238e1ed58d5fad9363d7439d2b5a8eeda6073d68ba87c0b10011',
      '02000000b90106bf4261e13389f867c267e86bd0015dcf9c48c784738695d0a3b3f8460c',
    ],
  },
  {
    keyVersion: 2,
    polynomial:
      '0200000000000000d9c90e58fdeaa61c9aa35478b3c5476d545d7a719c16c82c91087978232a030033889cd0019e96a3d0850f7d18861a00e81f754052106aa7c3bdd6e535e3ad303b2620a4c316dc598e0f9c659b50ceb0932d4da8c161046fc44f25cf05dde900ebf6f83c5cb94288347ebf437e99fbb7a7eaf0c9873467352c1a9113f5fc0974d96cbf25462def50c39224da757ed300ce12e0fa8c6e73387cb43c69764bed41d0a0c55981642650b07fad1107a27b27fc8c552da3edd64494e8acc4de9a2600',
    pubKey:
      '2ckOWP3qphyao1R4s8VHbVRdenGcFsgskQh5eCMqAwAziJzQAZ6Wo9CFD30YhhoA6B91QFIQaqfDvdblNeOtMDsmIKTDFtxZjg+cZZtQzrCTLU2owWEEb8RPJc8F3ekA',
    shares: [
      '0000000087c722e1338395b942d8332328795a46c718baeb8fef9e5c63111d495469c50e',
      '01000000e4efa9b60743f8188a68d35663d877143ad1726931eaa9af168fc86472eafd0d',
      '020000004118318cdb025b78d1f8728a9e3795e2ac892be7d2e4b402ca0c7480906b360d',
    ],
  },
  {
    keyVersion: 3,
    polynomial:
      '0200000000000000e68f58e75e9dbf364bcbb13f49f3929b6915874dadd6b535b6426b939e7f1e3851499b721d18003a76e7bca26f423000d4e13bd0bb1896b28af0f18d54ea7b71576b15b9bdc5b91ec3e054ea174ee3bdea89eac3385e128ca40d4e8ca1e543813fae8439a057f8c17d4538afecf038624e552a8c226c9f82bfb7a072cff28fb7d26ab45801b67db270cec8037b8d7e016b1b78f7997160bd4ed1b54ab5d6be7663935992cd9c59ceb17010eccd708a9762df616c1fe45a220be634e21ba87581',
    pubKey:
      '5o9Y516dvzZLy7E/SfOSm2kVh02t1rU1tkJrk55/HjhRSZtyHRgAOnbnvKJvQjAA1OE70LsYlrKK8PGNVOp7cVdrFbm9xbkew+BU6hdO473qierDOF4SjKQNToyh5UOB',
    shares: [
      '000000005b2c8089ead28a08233b6b16b2341542453523445950cfbd9bd2f1d09c8eee0c',
      '01000000f6c10aa979a0c33a3af5b03c37ffdf1d4a8517a5f9e6058e1d863337eeb59904',
      '02000000925795c808ee0d7752aff632bb40555350854362b8caf0bef5dea1379e42f00e',
    ],
  },
]

const keys = DEV_KEYS.map((key) => ({
  name: `dev key version ${key.keyVersion}`,
  keyVersion: key.keyVersion,
  threshold: 2,
  polynomial: key.polynomial,
  pubKey: key.pubKey,
  shares: key.shares,
}))
const fresh = bls.thresholdKeygen(5, 4, sha256('combiner 4 of 5'))
keys.push({
  name: '4 of 5',
  keyVersion: 4,
  threshold: 4,
  polynomial: hex(fresh.polynomial),
  pubKey: Buffer.from(fresh.thresholdPublicKey).toString('base64'),
  shares: Array.from({ length: fresh.numShares() }, (_, i) => hex(fresh.getShare(i))),
})

// Message 0 is the blinded message of
// odis-combiner-4.0.5:apps/combiner/test/integration/pnp.test.ts.
const seed0 = new Uint8Array(32)
for (let i = 0; i < 31; i++) seed0[i] = i
const messages = [
  hex(bls.blind(Buffer.from('test message'), seed0).message),
  ...[1, 2].map((i) =>
    hex(bls.blind(Buffer.from(`combine ${i}`), sha256(`combine blinding ${i}`)).message),
  ),
]

const partialVerifies = (key, message, partial) => {
  try {
    bls.partialVerifyBlindSignature(
      Buffer.from(keys[key].polynomial, 'hex'),
      Buffer.from(messages[message], 'hex'),
      Buffer.from(partial, 'hex'),
    )
    return true
  } catch (_) {
    return false
  }
}
const sign = (share, message) =>
  hex(bls.partialSignBlindedMessage(Buffer.from(share, 'hex'), Buffer.from(message, 'hex')))

const partials = []
const honest = {}
keys.forEach((key, k) => {
  key.shares.forEach((share, s) => {
    messages.forEach((message, m) => {
      const partial = sign(share, message)
      honest[`${k}/${s}/${m}`] = partial
      partials.push({
        key: k,
        share: s,
        message: m,
        partial,
        valid: partialVerifies(k, m, partial),
      })
    })
  })
})

// Invalid partials, all for key 0 and message 0.
const flipped = Buffer.from(honest['0/0/0'], 'hex')
flipped[20] ^= 0x01
const wrongIndex = Buffer.from(honest['0/0/0'], 'hex')
Buffer.from(honest['0/1/0'], 'hex').copy(wrongIndex, wrongIndex.length - 4, wrongIndex.length - 4)
for (const [partial, checkedAgainstKey] of [
  [hex(flipped), 0],
  [honest['1/0/0'], 0],
  [hex(wrongIndex), 0],
]) {
  partials.push({
    key: checkedAgainstKey,
    share: 0,
    message: 0,
    partial,
    valid: partialVerifies(checkedAgainstKey, 0, partial),
  })
}

const combined = []
keys.forEach((key, k) => {
  const n = key.shares.length
  for (let mask = 1; mask < 1 << n; mask++) {
    const shares = [...Array(n).keys()].filter((s) => mask & (1 << s))
    if (shares.length < key.threshold) continue
    messages.forEach((message, m) => {
      const sigs = Buffer.concat(shares.map((s) => Buffer.from(honest[`${k}/${s}/${m}`], 'hex')))
      const signature = bls.combine(key.threshold, sigs)
      bls.verifyBlindSignature(
        Buffer.from(key.pubKey, 'base64'),
        Buffer.from(message, 'hex'),
        signature,
      )
      combined.push({ key: k, message: m, shares, signature: hex(signature) })
    })
  }
})

const vectors = {
  source: `@celo/blind-threshold-bls ${version}; partials are checked with partialVerifyBlindSignature, combined signatures with combine and verifyBlindSignature`,
  keys,
  messages,
  partials,
  combined,
}
fs.writeFileSync(
  path.join(__dirname, 'combine_vectors.json'),
  JSON.stringify(vectors, null, 2) + '\n',
)
console.log(
  `${keys.length} keys, ${messages.length} messages, ${partials.length} partials, ${combined.length} combined signatures`,
)
