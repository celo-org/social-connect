import Logger, { createLogger, levelFromName, LogLevelString, stdSerializers } from 'bunyan'
import bunyanDebugStream from 'bunyan-debug-stream'
import { createStream } from 'bunyan-gke-stackdriver'

let _rootLogger: Logger | undefined

export function rootLogger(serviceName: string): Logger {
  if (_rootLogger !== undefined) {
    return _rootLogger
  }

  // `||` also falls back to the default when the variable is set but empty.
  const logLevel = (process.env.LOG_LEVEL || 'info') as LogLevelString
  const logFormat = process.env.LOG_FORMAT || 'human'

  let stream: any
  switch (logFormat) {
    case 'stackdriver':
      stream = createStream(levelFromName[logLevel])
      break
    case 'json':
      stream = { stream: process.stdout, level: logLevel }
      break
    default:
      stream = { level: logLevel, stream: bunyanDebugStream() }
      break
  }

  _rootLogger = createLogger({
    name: serviceName ?? '',
    serializers: stdSerializers,
    streams: [stream],
  })
  return _rootLogger
}

export function genSessionID() {
  return Math.random().toString(36).substring(2, 15) + Math.random().toString(36).substring(2, 15)
}
