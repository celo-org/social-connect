import threshold_bls from 'blind-threshold-bls'

require('dotenv').config()

export function getBlindedPhoneNumber(phoneNumber: string, blindingFactor: Buffer): string {
  const blindedPhoneNumber = threshold_bls.blind(Buffer.from(phoneNumber), blindingFactor).message
  return Buffer.from(blindedPhoneNumber).toString('base64')
}

export type E2ETestParams = {
  blockchainProviderURL: string
  pnpPolynomial: string
  pnpKeyVersion: string
}

export function getTestParamsForContext(): E2ETestParams {
  switch (process.env.CONTEXT_NAME) {
    case 'celo-sepolia':
      return {
        blockchainProviderURL: process.env.CELO_SEPOLIA_ODIS_BLOCKCHAIN_PROVIDER!,
        pnpPolynomial: process.env.CELO_SEPOLIA_PHONE_NUMBER_PRIVACY_POLYNOMIAL!,
        pnpKeyVersion: process.env.ODIS_PNP_TEST_KEY_VERSION!,
      }
    case 'mainnet':
      return {
        blockchainProviderURL: process.env.MAINNET_ODIS_BLOCKCHAIN_PROVIDER!,
        pnpPolynomial: process.env.MAINNET_PHONE_NUMBER_PRIVACY_POLYNOMIAL!,
        pnpKeyVersion: process.env.ODIS_PNP_TEST_KEY_VERSION!,
      }
    case 'local':
      return {
        blockchainProviderURL: process.env.CELO_SEPOLIA_ODIS_BLOCKCHAIN_PROVIDER!,
        pnpPolynomial: process.env.LOCAL_PHONE_NUMBER_PRIVACY_POLYNOMIAL!,
        pnpKeyVersion: process.env.ODIS_PNP_TEST_KEY_VERSION!,
      }
    default:
      throw new Error(`No parameter settings stored for context: ${process.env.CONTEXT_NAME}`)
  }
}
