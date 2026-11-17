type InputSurfaceToken = object;
type InputDeliveryRegistry = Map<string, InputSurfaceToken>;

function inputDeliveryRegistry(): InputDeliveryRegistry {
  const root = globalThis as typeof globalThis & {
    __LYNX_SSH_INPUT_DELIVERY_REGISTRY__?: InputDeliveryRegistry;
  };
  root.__LYNX_SSH_INPUT_DELIVERY_REGISTRY__ ??= new Map();
  return root.__LYNX_SSH_INPUT_DELIVERY_REGISTRY__;
}

/** Makes the newest mounted xterm surface the sole input owner for one PTY. */
export function activateSshTerminalInputSurface(
  terminalId: string,
  token: InputSurfaceToken
): void {
  const registry = inputDeliveryRegistry();
  registry.set(terminalId, token);
}

/** Rejects callbacks retained by an older React/HMR xterm surface. */
export function ownsSshTerminalInputSurface(terminalId: string, token: InputSurfaceToken): boolean {
  return inputDeliveryRegistry().get(terminalId) === token;
}

/** Releases ownership without allowing stale cleanup to evict a newer surface. */
export function releaseSshTerminalInputSurface(terminalId: string, token: InputSurfaceToken): void {
  const registry = inputDeliveryRegistry();
  if (registry.get(terminalId) === token) registry.delete(terminalId);
}

/** Returns only bytes not already represented by a terminal snapshot/event. */
export function unseenSshTerminalOutput(
  receivedBytes: number,
  eventReceivedBytes: number,
  data: number[]
): number[] {
  const eventEnd = Math.max(0, Math.trunc(eventReceivedBytes));
  const eventStart = Math.max(0, eventEnd - data.length);
  const unseenOffset = Math.max(0, Math.min(data.length, Math.trunc(receivedBytes) - eventStart));
  return data.slice(unseenOffset);
}
