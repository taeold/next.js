import { MessageChannel, Worker, type MessagePort } from 'worker_threads'

const loaderWorkers: Record<string, Map<number, Worker>> = {}

function getPoolId(cwd: string, filename: string) {
  return `${cwd}:${filename}`
}

// The calls a loader worker makes on the binding (see turbopack-node's worker_thread/taskChannel.ts).
const LOADER_WORKER_METHODS = new Set([
  'workerCreated',
  'sendTaskMessage',
  'recvTaskMessageInWorker',
])

// Runs a loader worker's binding calls on this thread's binding. See loaderWorkerBinding.ts.
function serveLoaderWorkerBinding(
  bindings: typeof import('./generated-native'),
  port: MessagePort
) {
  port.on('message', async ({ id, method, args }) => {
    if (!LOADER_WORKER_METHODS.has(method)) return
    try {
      const result = await (bindings as any)[method](...args)
      if (id !== undefined) port.postMessage({ id, result })
    } catch (error) {
      if (id !== undefined) {
        port.postMessage({
          id,
          error:
            error instanceof Error
              ? { message: error.message, stack: error.stack }
              : { message: String(error) },
        })
      }
    }
  })
  port.unref()
}

export async function runLoaderWorkerPool(
  bindings: typeof import('./generated-native'),
  bindingPath: string,
  bindingType: 'native' | 'wasi' = 'native'
) {
  bindings.registerWorkerScheduler(
    (creation) => {
      const {
        options: { filename, cwd },
      } = creation

      const poolId = getPoolId(cwd, filename)

      // A native addon is loaded once per process, so a worker that requires it reaches the running Turbopack. A
      // WASI binding is a wasm instance with its own memory: a second copy in the worker would be a second
      // Turbopack. The worker gets a binding that forwards its calls to this one over a MessagePort instead.
      let worker: Worker
      if (bindingType === 'wasi') {
        const { port1, port2 } = new MessageChannel()
        serveLoaderWorkerBinding(bindings, port1)
        worker = new Worker(/* turbopackIgnore: true*/ filename, {
          workerData: {
            bindingPath: require.resolve('./loaderWorkerBinding'),
            bindingPort: port2,
            cwd,
          },
          transferList: [port2],
        })
        worker.once('exit', () => port1.close())
      } else {
        worker = new Worker(/* turbopackIgnore: true*/ filename, {
          workerData: {
            bindingPath,
            cwd,
          },
        })
      }

      // This will cause handing when run in jest worker, but not as a first level thread of nodejs thread
      // worker.unref()

      const workers =
        loaderWorkers[poolId] || (loaderWorkers[poolId] = new Map())

      workers.set(worker.threadId, worker)
    },
    (termination) => {
      const {
        options: { filename, cwd },
        workerId,
      } = termination

      const poolId = getPoolId(cwd, filename)
      const workers = loaderWorkers[poolId]
      workers.get(workerId)?.terminate()
      workers.delete(workerId)
    }
  )
}
