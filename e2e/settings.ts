import BigNumber from 'bignumber.js'

// What a signer charges per query: the default of QUERY_PRICE_PER_CUSD in the signers.
export const QUERY_PRICE_IN_CUSD = new BigNumber(0.001)

// The version the service under test must report. The suites compare every response with it.
export function expectedServiceVersion(
  variable: 'DEPLOYED_SIGNER_SERVICE_VERSION' | 'DEPLOYED_COMBINER_SERVICE_VERSION',
): string {
  const version = process.env[variable]
  if (version === undefined || version === '') {
    throw new Error(`${variable} is not set`)
  }
  return version
}
