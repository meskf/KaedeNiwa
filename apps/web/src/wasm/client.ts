import init, { JsClient } from './pkg/kaede_client_wasm.js'

let client: JsClient | null = null
let initPromise: Promise<JsClient> | null = null

export function initWasm() {
  if (!initPromise) {
    initPromise = (async () => {
      await init()
      client = new JsClient()
      return client
    })()
  }
  return initPromise
}

export function getClient() {
  return client
}
