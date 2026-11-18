// The binding a Turbopack loader worker loads when Next.js runs on the WASI binding. It forwards the worker's calls
// to the binding on the thread that started the worker, over the MessagePort in workerData (see
// loaderWorkerPool.ts), because a second copy of the wasm binding would be a separate Turbopack.
import { workerData, type MessagePort } from 'worker_threads'

const port: MessagePort = workerData.bindingPort
const pending = new Map<
  number,
  { resolve: (value: any) => void; reject: (error: Error) => void }
>()
let nextId = 0

port.on('message', ({ id, result, error }) => {
  const call = pending.get(id)
  if (!call) return
  pending.delete(id)
  // Like a pending native call, a pending call keeps this thread alive.
  if (pending.size === 0) port.unref()
  if (error) call.reject(Object.assign(new Error(error.message), error))
  else call.resolve(result)
})
port.unref()

function call(method: string, args: unknown[]): Promise<any> {
  const id = nextId++
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject })
    port.ref()
    port.postMessage({ id, method, args })
  })
}

export function workerCreated(workerId: number): void {
  port.postMessage({ method: 'workerCreated', args: [workerId] })
}

export function sendTaskMessage(message: {
  taskId: number
  data: Uint8Array
}): void {
  port.postMessage({ method: 'sendTaskMessage', args: [message] })
}

export function recvTaskMessageInWorker(
  workerId: number
): Promise<{ taskId: number; data: Uint8Array }> {
  return call('recvTaskMessageInWorker', [workerId])
}
