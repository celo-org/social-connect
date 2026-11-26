describe('@celo/identity', () => {
  it('exports exactly these values from its root', () => {
    expect(Object.keys(require('./index')).sort()).toEqual([
      'AuthenticationMethod',
      'CombinerEndpoint',
      'ErrorMessage',
      'KEY_VERSION_HEADER',
      'OdisUtils',
      'PnpQuotaRequestSchema',
      'PnpQuotaResponseSchema',
      'SignMessageRequestSchema',
      'SignMessageResponseSchema',
      'SignerEndpoint',
      'WarningMessage',
      'getMessageDigest',
      'signWithRawKey',
    ])
  })
})
