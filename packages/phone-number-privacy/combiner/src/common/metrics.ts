import * as client from 'prom-client'
import { config } from '../config'

const { Counter, Histogram } = client

export const register = new client.Registry()

register.setDefaultLabels({
  app: config.serviceName,
})

client.collectDefaultMetrics({ register })

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
    labelNames: ['endpoint', 'error_type'],
  }),
  blockchainErrors: new Counter({
    name: 'combiner_blockchain_errors_total',
    help: 'Counter for the number of errors from interacting with the blockchain',
    labelNames: ['endpoint', 'error_type'],
  }),
  blsComputeErrors: new Counter({
    name: 'combiner_bls_compute_errors_total',
    help: 'Counter for the number of BLS compute errors',
    labelNames: ['endpoint', 'signer'],
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

register.registerMetric(Counters.requests)
register.registerMetric(Counters.responses)
register.registerMetric(Counters.errors)
register.registerMetric(Counters.blockchainErrors)
register.registerMetric(Counters.blsComputeErrors)
register.registerMetric(Counters.errorsCaughtInEndpointHandler)
register.registerMetric(Counters.notEnoughSigErrors)
register.registerMetric(Counters.sigRequestErrors)
register.registerMetric(Counters.sigInconsistenciesErrors)
register.registerMetric(Counters.sigResponses)
register.registerMetric(Counters.unknownErrors)
register.registerMetric(Counters.warnings)

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
  eventLoopLag: new Histogram({
    name: 'combiner_event_loop_lag',
    help: 'Histogram event loop lag in Combiner',
    labelNames: ['endpoint'],
    buckets,
  }),
  signatureAggregationLatency: new Histogram({
    name: 'combiner_signature_aggregation_latency',
    help: 'Histogram latency of signature aggregation in Combiner',
    labelNames: ['endpoint'],
    buckets,
  }),
  signerTailLatency: new Histogram({
    name: 'signer_tail_latency',
    help: 'Histogram of latency discrepencies between the fastest and slowest signer in a quorum',
    labelNames: ['endpoint'],
    buckets,
  }),
}

register.registerMetric(Histograms.responseLatency)
register.registerMetric(Histograms.fullNodeLatency)
register.registerMetric(Histograms.signerLatency)
register.registerMetric(Histograms.eventLoopLag)
register.registerMetric(Histograms.signatureAggregationLatency)
register.registerMetric(Histograms.signerTailLatency)

export function newMeter(
  histogram: client.Histogram<string>,
  ...labels: string[]
): <U>(fn: () => Promise<U>) => Promise<U> {
  return (fn) => {
    const _meter = histogram.labels(...labels).startTimer()
    return fn().finally(_meter)
  }
}
