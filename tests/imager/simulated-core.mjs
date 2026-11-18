export * from '../../node_modules/@tauri-apps/api/core.js';
import { invoke as nativeInvoke } from '../../node_modules/@tauri-apps/api/core.js';
const sim = {
  usb: [],
  profiles: [],
  tasks: [],
  calls: [],
  responses: {},
  nextFile: null,
  lastId: '',
  sequence: 0,
};
window.__imagerSimulation = sim;
export async function invoke(cmd, args = {}, options) {
  sim.calls.push({ cmd, args });
  if (Object.hasOwn(sim.responses, cmd)) return sim.responses[cmd];
  if (cmd === 'plugin:dialog|open') {
    const p = sim.nextFile;
    sim.nextFile = null;
    return p;
  }
  if (cmd === 'plugin:dialog|save') return null;
  if (cmd === 'workbench_list_devices') return sim.profiles;
  if (cmd === 'workbench_save_device') {
    const item = { ...args.device, id: Math.max(0, ...sim.profiles.map((d) => d.id)) + 1 };
    sim.profiles.push(item);
    return item.id;
  }
  if (cmd === 'workbench_scan_flasher_devices')
    return sim.usb.filter((d) => d.flasherKind === args.kind);
  if (cmd === 'workbench_list_tasks') return sim.tasks;
  if (cmd === 'workbench_start_flash' || cmd === 'workbench_firmware_start') {
    sim.lastId = 'sim-job-' + ++sim.sequence;
    sim.tasks = [
      {
        taskId: sim.lastId,
        deviceId: args.deviceId,
        status: 'running',
        phase: 'preparing',
        error: '',
        details: {},
        startedAt: Math.floor(Date.now() / 1000),
        progressPct: 0,
      },
    ];
    return sim.lastId;
  }
  if (/^(workbench_|usb_|adb_|serial_|openocd_|mass_|ch34)/.test(cmd))
    throw Error('No hardware fixture for ' + cmd);
  return nativeInvoke(cmd, args, options);
}
