import { useCallback, useRef, useState, type SetStateAction } from 'react';
import type { DeviceFlowTemplate } from './deviceFlowModel';

const HISTORY_LIMIT = 50;

const sameFlow = (left: DeviceFlowTemplate, right: DeviceFlowTemplate) =>
  left === right || JSON.stringify(left) === JSON.stringify(right);

export const useDeviceFlowHistory = (initialFlow: DeviceFlowTemplate) => {
  const [flow, setFlow] = useState(initialFlow);
  const historyRef = useRef<DeviceFlowTemplate[]>([]);
  const futureRef = useRef<DeviceFlowTemplate[]>([]);
  const [, setVersion] = useState(0);

  const refreshAvailability = useCallback(() => setVersion((value) => value + 1), []);

  const reset = useCallback(
    (next: DeviceFlowTemplate) => {
      setFlow(next);
      historyRef.current = [];
      futureRef.current = [];
      refreshAvailability();
    },
    [refreshAvailability]
  );

  const replace = useCallback((next: SetStateAction<DeviceFlowTemplate>) => {
    setFlow(next);
  }, []);

  const commit = useCallback(
    (next: DeviceFlowTemplate) => {
      if (!sameFlow(next, flow)) {
        historyRef.current = [...historyRef.current.slice(-49), flow];
        futureRef.current = [];
        refreshAvailability();
      }
      setFlow(next);
    },
    [flow, refreshAvailability]
  );

  const recordSnapshot = useCallback(
    (snapshot: DeviceFlowTemplate) => {
      historyRef.current = [...historyRef.current.slice(-49), snapshot];
      futureRef.current = [];
      refreshAvailability();
    },
    [refreshAvailability]
  );

  const undo = useCallback(() => {
    const previous = historyRef.current[historyRef.current.length - 1];
    if (!previous) return undefined;
    historyRef.current = historyRef.current.slice(0, -1);
    futureRef.current = [flow, ...futureRef.current.slice(0, HISTORY_LIMIT - 1)];
    setFlow(previous);
    refreshAvailability();
    return previous;
  }, [flow, refreshAvailability]);

  const redo = useCallback(() => {
    const next = futureRef.current[0];
    if (!next) return undefined;
    futureRef.current = futureRef.current.slice(1);
    historyRef.current = [...historyRef.current.slice(-49), flow];
    setFlow(next);
    refreshAvailability();
    return next;
  }, [flow, refreshAvailability]);

  return {
    flow,
    reset,
    replace,
    commit,
    recordSnapshot,
    undo,
    redo,
    canUndo: historyRef.current.length > 0,
    canRedo: futureRef.current.length > 0,
  };
};
