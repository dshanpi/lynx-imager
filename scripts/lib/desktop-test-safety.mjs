// Test-only cleanup guard. Never cancel hardware to make cleanup possible.
export async function ensureDesktopIdle(invoke) {
  const hardware = await invoke('hardware_safety_status', {});
  if (!hardware || typeof hardware.busy !== 'boolean' || !Array.isArray(hardware.activities)) {
    throw new Error('SAFETY_STATE_UNAVAILABLE: retaining test application and data');
  }
  if (hardware.busy || hardware.activities.length) {
    throw new Error('HARDWARE_TASK_ACTIVE: retaining test application and data');
  }
  const tasks = await invoke('workbench_list_tasks', { limit: 1000 });
  if (!Array.isArray(tasks)) throw new Error('SAFETY_STATE_UNAVAILABLE: persisted task state');
  if (tasks.some((task) => task.status === 'running')) {
    throw new Error('HARDWARE_TASK_ACTIVE: persisted task is running');
  }
  return { hardware, tasks };
}
