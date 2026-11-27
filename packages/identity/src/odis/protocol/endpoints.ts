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
