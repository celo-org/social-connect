import { Server } from 'http'
import { Server as HttpsServer } from 'https'

export async function serverClose(server?: Server | HttpsServer) {
  if (server) {
    // Force close all connections before closing the server
    if ('closeAllConnections' in server && typeof server.closeAllConnections === 'function') {
      server.closeAllConnections()
    }
    await new Promise((resolve, reject) => {
      server.close((err) => {
        if (err) reject(err)
        else resolve(undefined)
      })
    })
  }
}
