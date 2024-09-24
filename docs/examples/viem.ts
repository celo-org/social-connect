import { federatedAttestationsABI, odisPaymentsABI, stableTokenABI } from '@celo/abis'
import { OdisUtils } from '@celo/identity'
import { AuthSigner, OdisContextName, ServiceContext } from '@celo/identity/lib/odis/query'
import {
  Address,
  createPublicClient,
  createWalletClient,
  getContract,
  Hex,
  http,
  parseEther,
  PrivateKeyAccount,
  PublicClient,
  Transport,
  WalletClient,
} from 'viem'
import { readContract } from 'viem/_types/actions/public/readContract'
import { privateKeyToAccount } from 'viem/accounts'
import { celoAlfajores } from 'viem/chains'
import {
  ALFAJORES_CUSD_ADDRESS,
  FA_PROXY_ADDRESS as FEDERATED_ATTESTATIONS_ADDRESS,
  ODIS_PAYMENTS_PROXY_ADDRESS,
} from './constants'

const ALFAJORES_RPC = 'https://alfajores-forno.celo-testnet.org'
const ISSUER_PRIVATE_KEY = '0x199abda8320f5af0bb51429d246a4e537d1c85fbfaa30d52f9b34df381bd3a95'
class ASv2 {
  private walletClient: WalletClient
  private publicClient: PublicClient<Transport, typeof celoAlfajores>
  issuer: PrivateKeyAccount
  authSigner: AuthSigner
  serviceContext: ServiceContext

  constructor() {
    this.issuer = privateKeyToAccount(ISSUER_PRIVATE_KEY)

    this.walletClient = createWalletClient({
      chain: celoAlfajores,
      transport: http(ALFAJORES_RPC),
      account: this.issuer,
    })
    this.publicClient = createPublicClient({ chain: celoAlfajores, transport: http(ALFAJORES_RPC) })

    this.serviceContext = OdisUtils.Query.getServiceContext(OdisContextName.ALFAJORES)
    this.authSigner = {
      authenticationMethod: OdisUtils.Query.AuthenticationMethod.WALLET_KEY,
      client: this.walletClient,
    }
  }

  async registerAttestation(phoneNumber: string, account: Address, attestationIssuedTime: bigint) {
    await this.checkAndTopUpODISQuota()

    // get identifier from phone number using ODIS
    const { obfuscatedIdentifier } = await OdisUtils.Identifier.getObfuscatedIdentifier(
      phoneNumber,
      OdisUtils.Identifier.IdentifierPrefix.PHONE_NUMBER,
      this.issuer.address,
      this.authSigner,
      this.serviceContext,
    )

    const federatedAttestations = getContract({
      abi: federatedAttestationsABI,
      address: FEDERATED_ATTESTATIONS_ADDRESS,
      client: this.walletClient,
    })

    await federatedAttestations.write.registerAttestationAsIssuer(
      [obfuscatedIdentifier as Hex, account, attestationIssuedTime],
      { chain: this.walletClient.chain, account: this.issuer },
    )

    // const { request } = await simulateContract(this.walletClient, {
    //   abi: federatedAttestationsABI,
    //   functionName: 'registerAttestationAsIssuer',
    //   address: FEDERATED_ATTESTATIONS_ADDRESS,
    //   chain: celoAlfajores,
    // })

    // await this.walletClient.writeContract(request)
  }

  async lookupAddresses(phoneNumber: string) {
    // get identifier from phone number using ODIS
    const { obfuscatedIdentifier } = await OdisUtils.Identifier.getObfuscatedIdentifier(
      phoneNumber,
      OdisUtils.Identifier.IdentifierPrefix.PHONE_NUMBER,
      this.issuer.address,
      this.authSigner,
      this.serviceContext,
    )

    // query on-chain mappings
    const [_countsPerIssuer, accounts, _signers] = await readContract(this.publicClient, {
      abi: federatedAttestationsABI,
      functionName: 'lookupAttestations',
      address: '0x',
      args: [obfuscatedIdentifier as Hex, [this.issuer.address]],
    })

    return accounts
  }

  private async checkAndTopUpODISQuota() {
    //check remaining quota
    const { remainingQuota } = await OdisUtils.Quota.getPnpQuotaStatus(
      this.issuer.address,
      this.authSigner,
      this.serviceContext,
    )

    console.log('remaining ODIS quota', remainingQuota)
    if (remainingQuota < 1) {
      const stableTokenContract = getContract({
        abi: stableTokenABI,
        address: ALFAJORES_CUSD_ADDRESS,
        client: { public: this.publicClient, wallet: this.walletClient },
      })
      const odisPaymentsContract = getContract({
        abi: odisPaymentsABI,
        address: ODIS_PAYMENTS_PROXY_ADDRESS,
        client: { public: this.publicClient, wallet: this.walletClient },
      })

      // give odis payment contract permission to use cUSD
      const currentAllowance = await stableTokenContract.read.allowance([
        this.issuer.address,
        odisPaymentsContract.address,
      ])
      console.log('current allowance:', currentAllowance.toString())
      let enoughAllowance: boolean = false

      const ONE_CENT_CUSD_WEI = parseEther('0.01')

      if (currentAllowance <= ONE_CENT_CUSD_WEI) {
        const approvalTxHash = await stableTokenContract.write.increaseAllowance(
          [odisPaymentsContract.address, ONE_CENT_CUSD_WEI],
          { account: this.issuer, chain: celoAlfajores },
        )

        const approvalTxReceipt = await this.publicClient.waitForTransactionReceipt({
          hash: approvalTxHash,
        })

        console.log('approval status', approvalTxReceipt.status)
        enoughAllowance = approvalTxReceipt.status === 'success'
      } else {
        enoughAllowance = true
      }

      // increase quota
      if (enoughAllowance) {
        const odisPaymentHash = await odisPaymentsContract.write.payInCUSD(
          [this.issuer.address, ONE_CENT_CUSD_WEI],
          { account: this.issuer, chain: celoAlfajores },
        )

        const odisPaymentReceipt = await this.publicClient.waitForTransactionReceipt({
          hash: odisPaymentHash,
        })
        console.log('odis payment tx status:', odisPaymentReceipt.status)
        console.log('odis payment tx hash:', odisPaymentHash)
      } else {
        throw 'cUSD approval failed'
      }
    }
  }
}

;(async () => {
  const asv2 = new ASv2()
  const userAccount = '0xf14790BAdd2638cECB5e885fc7fAD1b6660AAc34'
  const userPhoneNumber = '+18009099999'
  const timeAttestationWasVerified = BigInt(Math.floor(new Date().getTime() / 1000))
  try {
    await asv2.registerAttestation(userPhoneNumber, userAccount, timeAttestationWasVerified)
    console.log('attestation registered')
  } catch (err) {
    // mostly likely reason registering would fail is if this issuer has already
    // registered a mapping between this number and account
  }
  console.log(await asv2.lookupAddresses(userPhoneNumber))
})()
