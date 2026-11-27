import { getMessageDigest, signWithRawKey } from './authentication'

// The DEK key the ODIS end-to-end tests register for their account.
const DEK_PRIVATE_KEY = '0xbf8a2b73baf8402f8fe906ad3f42b560bf14b39f7df7797ece9e293d6f162188'

// TS signers check DEK signatures over exactly these bytes, so neither output may change.
describe('DEK request authentication', () => {
  it('signs the message digest as a JSON array of DER bytes', () => {
    expect(signWithRawKey('odis protocol vector', DEK_PRIVATE_KEY)).toEqual(
      '[48,69,2,33,0,215,154,41,223,123,118,41,212,34,67,163,158,104,182,157,192,171,135,201,176,183,119,136,100,128,249,218,27,243,56,166,215,2,32,101,138,95,152,172,138,105,207,10,97,29,225,7,11,222,184,33,132,193,122,222,136,199,237,233,101,1,56,182,238,122,5]',
    )
  })

  it('digests the JSON-encoded message with SHA-256', () => {
    expect(getMessageDigest('odis protocol vector')).toEqual(
      'd1ce3f7c9da98382e61951a547dbd49ae8d46a3d3173f79a081c7f1f014392b6',
    )
  })
})
