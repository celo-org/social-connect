import * as client from 'prom-client'

const { Counter, Histogram } = client

client.collectDefaultMetrics()

// This is just so autocomplete will remind devs what the options are.
export enum Labels {
  READ = 'read',
  UPDATE = 'update',
  INSERT = 'insert',
  BATCH_DELETE = 'batch-delete',
}

export const Counters = {
  requests: new Counter({
    name: 'combiner_requests_total',
    help: 'Counter for the number of requests received',
    labelNames: ['endpoint'],
  }),
  responses: new Counter({
    name: 'combiner_responses_total',
    help: 'Counter for the number of responses sent',
    labelNames: ['endpoint', 'statusCode'],
  }),
  errors: new Counter({
    name: 'combiner_errors_total',
    help: 'Counter for the total number of errors',
    labelNames: ['endpoint'],
  }),
  blockchainErrors: new Counter({
    name: 'combiner_blockchain_errors_total',
    help: 'Counter for the number of errors from interacting with the blockchain',
  }),
  blsComputeErrors: new Counter({
    name: 'combiner_bls_compute_errors_total',
    help: 'Counter for the number of BLS compute errors',
    labelNames: ['signer'],
  }),
  errorsCaughtInEndpointHandler: new Counter({
    name: 'combiner_endpoint_handler_errors_total',
    help: 'Counter for the number of errors caught in the outermost endpoint handler',
    labelNames: ['endpoint'],
  }),
  notEnoughSigErrors: new Counter({
    name: 'combiner_not_enough_sig_errors_total',
    help: 'Counter for the number of not enough sig errors',
    labelNames: ['endpoint'],
  }),
  sigRequestErrors: new Counter({
    name: 'combiner_sig_request_errors_total',
    help: 'Counter for errors receiving signer request (not triggered by combiner abort)',
    labelNames: ['signer', 'endpoint', 'error_type'],
  }),
  sigResponsesErrors: new Counter({
    name: 'combiner_sig_response_errors_total',
    help: 'Counter for error responses received from signers',
    labelNames: ['status', 'signer', 'endpoint'],
  }),
  sigInconsistenciesErrors: new Counter({
    name: 'combiner_sig_inconsistency_errors_total',
    help: 'Counter for signer inconsistency errors',
    labelNames: ['endpoint'],
  }),
  sigResponses: new Counter({
    name: 'combiner_sig_response_total',
    help: 'Counter for responses received from signers',
    labelNames: ['status', 'signer', 'endpoint'],
  }),
  unknownErrors: new Counter({
    name: 'combiner_unknown_errors_total',
    help: 'Counter for unknown errors thrown in the combiner',
    labelNames: ['endpoint'],
  }),
  warnings: new Counter({
    name: 'combiner_warnings_total',
    help: 'Counter for all cpmbiner warnings',
    labelNames: ['endpoint', 'warning_type'],
  }),
}

const buckets = [0.001, 0.01, 0.1, 0.5, 1, 2, 5, 10]

export const Histograms = {
  responseLatency: new Histogram({
    name: 'combiner_endpoint_latency',
    help: 'Histogram tracking latency of combiner endpoints',
    labelNames: ['endpoint'],
    buckets,
  }),
  fullNodeLatency: new Histogram({
    name: 'combiner_full_node_latency',
    help: 'Histogram tracking latency of full node requests',
    labelNames: ['codeSegment'],
    buckets,
  }),
  signerLatency: new Histogram({
    name: 'combiner_signer_latency',
    help: 'Histogram tracking latency of signers',
    labelNames: ['endpoint', 'signer'],
    buckets,
  }),
}

export function newMeter(
  histogram: client.Histogram<string>,
  ...labels: string[]
): <U>(fn: () => Promise<U>) => Promise<U> {
  return (fn) => {
    const _meter = histogram.labels(...labels).startTimer()
    return fn().finally(_meter)
  }
}
