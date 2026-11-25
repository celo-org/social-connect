import { hexToBuffer } from '@celo/base'
import crypto from 'crypto'

export function getMessageDigest(message: string) {
  // NOTE: Elliptic will truncate the raw msg to 64 bytes before signing,
  // so make sure to always pass the hex encoded msgDigest instead.
  return crypto.createHash('sha256').update(JSON.stringify(message)).digest('hex')
}

// Used primarily for signing requests with a DEK, counterpart of the signers' DEK signature check
// For general signing, use SignatureUtils in @celo/utils
export function signWithRawKey(msg: string, rawKey: string) {
  // NOTE: elliptic is disabled elsewhere in this library to prevent
  // accidental signing of truncated messages.
  const EC = require('elliptic').ec
  const ec = new EC('secp256k1')

  // Sign
  const key = ec.keyFromPrivate(hexToBuffer(rawKey))
  return JSON.stringify(key.sign(getMessageDigest(msg)).toDER())
}
