import { useEffect, useState } from 'react';
import { invokeCommand, subscribeEvent } from '../../Platform/IPC';

type Presence = 'online' | 'offline' | 'unknown';

/** Observe existing terminal sessions without opening SSH or subscribing to terminal output. */
export function useSshPresence(
  active: boolean,
  projectId: number | null,
  deviceId: number | null
): Presence {
  const [view, setView] = useState<{
    projectId: number;
    deviceId: number;
    presence: Presence;
  } | null>(null);
  useEffect(() => {
    setView(null);
    if (!active || projectId == null || deviceId == null) return;
    let current = true;
    let snapshotState: 'loading' | 'ready' | 'failed' = 'loading';
    let release: (() => void) | undefined;
    const connected = new Set<string>();
    // Events received while reading the initial snapshot take precedence for those terminals.
    const changedDuringLoad = new Set<string>();
    function publish() {
      if (current && projectId != null && deviceId != null)
        setView({
          projectId,
          deviceId,
          presence: connected.size ? 'online' : snapshotState === 'ready' ? 'offline' : 'unknown',
        });
    }
    async function observe(projectId: number, deviceId: number) {
      const stop = await subscribeEvent('workbench-ssh-terminal-status', (event) => {
        if (!current || event.projectId !== projectId || event.deviceId !== deviceId) return;
        if (snapshotState === 'loading') changedDuringLoad.add(event.terminalId);
        if (event.state === 'connected') connected.add(event.terminalId);
        else connected.delete(event.terminalId);
        publish();
      });
      if (!current) {
        stop();
        return;
      }
      release = stop;
      const sessions = await invokeCommand('workbench_ssh_terminal_list', { projectId, deviceId });
      if (!current) return;
      for (const session of sessions) {
        if (
          session.projectId === projectId &&
          session.deviceId === deviceId &&
          session.state === 'connected' &&
          !changedDuringLoad.has(session.terminalId)
        ) {
          connected.add(session.terminalId);
        }
      }
      snapshotState = 'ready';
      changedDuringLoad.clear();
      publish();
    }
    void observe(projectId, deviceId).catch(() => {
      snapshotState = 'failed';
      changedDuringLoad.clear();
      publish();
    });
    return () => {
      current = false;
      release?.();
    };
  }, [active, projectId, deviceId]);
  if (!active || !view || view.deviceId !== deviceId || view.projectId !== projectId)
    return 'unknown';
  return view.presence;
}
