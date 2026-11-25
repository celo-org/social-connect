export enum SignerEndpointCommon {
  METRICS = '/metrics',
  STATUS = '/status',
}

export enum SignerEndpointPNP {
  PNP_QUOTA = '/quotaStatus',
  PNP_SIGN = '/sign',
}

export enum CombinerEndpointCommon {
  METRICS = '/metrics',
  STATUS = '/status',
}

export enum CombinerEndpointPNP {
  PNP_QUOTA = '/quotaStatus',
  PNP_SIGN = '/sign',
  STATUS = '/status',
}

export type SignerEndpoint = SignerEndpointCommon | SignerEndpointPNP
export const SignerEndpoint = { ...SignerEndpointCommon, ...SignerEndpointPNP }

export type CombinerEndpoint = CombinerEndpointCommon | CombinerEndpointPNP
export const CombinerEndpoint = {
  ...CombinerEndpointCommon,
  ...CombinerEndpointPNP,
}

export type Endpoint = SignerEndpoint | CombinerEndpoint
export const Endpoint = { ...SignerEndpoint, ...CombinerEndpoint }

export function getSignerEndpoint(endpoint: CombinerEndpoint): SignerEndpoint {
  switch (endpoint) {
    case CombinerEndpoint.PNP_QUOTA:
      return SignerEndpoint.PNP_QUOTA
    case CombinerEndpoint.PNP_SIGN:
      return SignerEndpoint.PNP_SIGN
    default:
      throw new Error(`No corresponding signer endpoint exists for combiner endpoint ${endpoint}`)
  }
}

export function getCombinerEndpoint(endpoint: SignerEndpoint): CombinerEndpoint {
  switch (endpoint) {
    case SignerEndpoint.PNP_QUOTA:
      return CombinerEndpoint.PNP_QUOTA
    case SignerEndpoint.PNP_SIGN:
      return CombinerEndpoint.PNP_SIGN
    default:
      throw new Error(`No corresponding combiner endpoint exists for signer endpoint ${endpoint}`)
  }
}
