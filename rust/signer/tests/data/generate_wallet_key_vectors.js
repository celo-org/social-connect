// Writes wallet_key_vectors.json: wallet-key signatures over request bodies, made
// the way the TS services verify them and the way EIP-191 says. The Rust signer's
// tests assert that it accepts them, because a request the TS combiner accepts
// must get the same answer from a Rust signer as from a TS one.
//
// Both ways hash "\x19Ethereum Signed Message:\n" + length + body, encoded as
// UTF-8. They differ in the length: @celo/utils writes the length of the
// JavaScript string, in UTF-16 code units, and EIP-191 the number of bytes. The
// two are equal only for an ASCII body.
//
// Run from the repository root, with the TS dependencies installed:
//   node rust/signer/tests/data/generate_wallet_key_vectors.js
const fs = require('fs')
const path = require('path')
const {
  hashMessageWithPrefix,
  serializeSignature,
  signMessage,
  verifySignature,
} = require('@celo/utils/lib/signatureUtils')
const { version: celoUtilsVersion } = require('@celo/utils/package.json')
const { hashMessage, keccak256, stringToBytes } = require('viem')
const { privateKeyToAccount } = require('viem/accounts')
const { version: viemVersion } = require('viem/package.json')

// A well-known test key. Never use it for anything of value.
const PRIVATE_KEY = '0x4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318'
const account = privateKeyToAccount(PRIVATE_KEY)

const body = (sessionID) =>
  JSON.stringify({
    account: account.address,
    blindedQueryPhoneNumber: 'n/I9srniwEHm5o6t3y0tTUB5fn7xjxRrLP1F/i8ORCdqV++WWiaAzUo3GA2UNHiB',
    sessionID,
  })
const bodies = [
  ['ASCII only', body('session')],
  ['a two-byte character', body('sé')],
  ['a three-byte character', body('s€')],
  ['a character outside the BMP, a surrogate pair in UTF-16', body('s😀')],
  ['all of them', body('sé€😀')],
]

const prefixedHash = (message, length) =>
  keccak256(stringToBytes('\x19Ethereum Signed Message:\n' + length + message))

async function main() {
  const vectors = []
  for (const [description, message] of bodies) {
    const utf16Length = message.length
    const byteLength = Buffer.byteLength(message, 'utf8')

    // Pin down what each library hashes, so the vectors mean what the header says.
    if (hashMessageWithPrefix(message) !== prefixedHash(message, utf16Length)) {
      throw new Error(`@celo/utils does not hash UTF-8 bytes with a UTF-16 length: ${description}`)
    }
    if (hashMessage(message) !== prefixedHash(message, byteLength)) {
      throw new Error(`viem does not hash UTF-8 bytes with a byte length: ${description}`)
    }

    // As @celo/utils `LocalSigner` signs: v || r || s.
    const celoUtilsSignature = serializeSignature(
      signMessage(message, PRIVATE_KEY, account.address),
    )
    // As viem, and so the SDK's `sign191` with a viem account, signs: r || s || v.
    const standardSignature = await account.signMessage({ message })

    if (!verifySignature(message, celoUtilsSignature, account.address)) {
      throw new Error(`the TS services refuse their own signature: ${description}`)
    }
    vectors.push({
      description,
      body: message,
      utf16Length,
      byteLength,
      celoUtilsSignature,
      standardSignature,
      tsAcceptsStandardSignature: verifySignature(message, standardSignature, account.address),
    })
  }

  const output = {
    source: `@celo/utils ${celoUtilsVersion} signMessage and verifySignature, viem ${viemVersion} signMessage`,
    account: account.address,
    vectors,
  }
  fs.writeFileSync(
    path.join(__dirname, 'wallet_key_vectors.json'),
    JSON.stringify(output, null, 2) + '\n',
  )
  for (const vector of vectors) {
    console.log(
      `${vector.description}: ${vector.utf16Length} UTF-16 units, ${vector.byteLength} bytes, ` +
        `TS accepts the standard signature: ${vector.tsAcceptsStandardSignature}`,
    )
  }
}

main()
