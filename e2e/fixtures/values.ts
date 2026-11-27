import { normalizeAddressWith0x, privateKeyToAddress } from '@celo/utils/lib/address'
import { Address } from 'viem'

export const PRIVATE_KEY1 = '535029bfb19fe5440dbd549b88fbf5ee847b059485e4eafc2a3e3bdfbf9b31ac'
export const ACCOUNT_ADDRESS1 = normalizeAddressWith0x(privateKeyToAddress(PRIVATE_KEY1)) as Address
export const PHONE_NUMBER = '+15555555555'
// DEK values shared by combiner and signer E2E tests to avoid conflicts
export const DEK_PUBLIC_KEY = '0x034846bc781cacdafc66f3a77aa9fc3c56a9dadcd683c72be3c446fee8da041070'
export const DEK_PRIVATE_KEY = '0xbf8a2b73baf8402f8fe906ad3f42b560bf14b39f7df7797ece9e293d6f162188'
