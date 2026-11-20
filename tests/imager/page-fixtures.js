// In-memory IPC for browser interaction tests. Loaded into an isolated test context only.
mockWindows('main');
mockConvertFileSrc('windows');
const qa = (window.pageFixture = {
  calls: [],
  tasks: [],
  providers: [],
  policies: [],
  devices: [],
  usb: [],
  nextFile: null,
  failRecords: false,
  failDiagnostics: false,
  failSettings: false,
});
if (sessionStorage.qaBoard) qa.devices = [JSON.parse(sessionStorage.qaBoard)];
mockIPC(
  async (cmd, args) => {
    qa.calls.push(cmd);
    switch (cmd) {
      case 'plugin:app|version':
        return '0.1.0-rc.2';
      case 'get_system_proxy':
        return null;
      case 'plugin:updater|check':
        if (qa.updateError) throw Error('QA update offline');
        return qa.update || null;
      case 'plugin:fs|exists':
        return !args.path.endsWith('settings.json') || localStorage.qaSettings != null;
      case 'plugin:fs|write_text_file':
        if (qa.exportPending) {
          qa.exported = JSON.parse(new TextDecoder().decode(args));
          qa.exportPending = false;
          return;
        }
        if (qa.failSettings) throw Error('test settings write failed');
        if (qa.holdSave)
          await new Promise((resolve) => {
            qa.finishSave = resolve;
          });
        localStorage.qaSettings = new TextDecoder().decode(args);
        return;
      case 'plugin:fs|read_text_file':
        return Array.from(new TextEncoder().encode(localStorage.qaSettings));
      case 'workbench_list_devices':
        return qa.devices;
      case 'workbench_list_projects':
        return qa.projects || [];
      case 'gateway_channel_status':
        return qa.channel;
      case 'gateway_list_interactions':
        return [];
      case 'workbench_list_tunnel_statuses':
        return qa.tunnels || [];
      case 'plugin:dialog|open':
        qa.lastDialog = args.options;
        return qa.nextFile;
      case 'plugin:dialog|save':
        qa.exportPending = true;
        return 'C:/qa/exported-record.json';
      case 'workbench_scan_flasher_devices':
        return qa.usb.filter((d) => d.flasherKind === args.kind);
      case 'imager_file_identity':
        return {
          sizeBytes: args.path.endsWith('/system.img') ? 614400 : 1024,
          sha256: (args.path.endsWith('/loader.img') ? '2b' : '1a').repeat(32),
        };
      case 'imager_inspect_image': {
        const loader = args.path.endsWith('/loader.img');
        return {
          knownF101Loader: false,
          sizeBytes: loader ? 1024 : 614400,
          sha256: (loader ? '2b' : '1a').repeat(32),
          headerHex: '00 00',
          appHex: '01',
          egonAtZero: !loader,
          splAt8192: false,
          containerVersion: loader ? 768 : null,
          entries: loader
            ? [
                {
                  name: 'fes1.fex',
                  mainType: 'FES',
                  subtype: 'FES_1-0000000000',
                  sizeBytes: 16,
                  offset: 512,
                },
                {
                  name: 'u-boot.fex',
                  mainType: '12345678',
                  subtype: 'UBOOT_0000000000',
                  sizeBytes: 16,
                  offset: 768,
                },
              ]
            : [],
        };
      }
      case 'imager_read_image_reference':
        return {
          documentSha256: 'ab'.repeat(32),
          sha256: '1a'.repeat(32),
          sizeBytes: 614400,
          applicationOffset: 524288,
          applicationBytes: 256,
          boardModel: null,
          fileName: 'system.img',
        };
      case 'imager_firmware_catalog':
        return [];
      case 'imager_inspect_vendor_image':
        return {
          route: args.route,
          path: args.path,
          bytes: 1024,
          format: 'RKFW',
          items: [],
          alternates: [],
          dependencies: [],
          blockers: [],
        };
      case 'workbench_start_flash': {
        qa.tasks = [
          {
            taskId: 'qa-flash',
            deviceId: args.deviceId,
            kind: 'flash',
            status: 'running',
            phase: 'preparing',
            startedAt: 100,
            details: {},
            progressPct: 0,
          },
        ];
        return 'qa-flash';
      }
      case 'workbench_list_tasks':
        if (qa.failRecords) throw Error('test history unavailable');
        return qa.tasks;
      case 'workbench_list_workflow_steps':
        return [{ id: 1, startedAt: 100, stepName: args.taskId, status: 'completed' }];
      case 'supervisor_list_providers':
        return qa.providers;
      case 'supervisor_list_policies':
        return qa.policies;
      case 'supervisor_save_provider': {
        const item = { ...args.request, id: args.request.id ?? 1 };
        qa.providers = [item];
        return item;
      }
      case 'supervisor_test_provider':
        return { ok: true, latencyMs: 12 };
      case 'supervisor_save_policy': {
        const item = { ...args.policy, id: args.policy.id ?? 1 };
        qa.policies = [item];
        return item;
      }
      case 'diagnostics_get_status':
        if (qa.failDiagnostics) throw Error('test diagnostics unavailable');
        return { enabled: false, state: 'disabled', pendingCount: 0 };
      case 'cloud_sync_get_status':
        return { enabled: false, state: 'disabled' };
      case 'plugin:opener|open_url':
        qa.url = args.url;
        return;
      case 'auth_get_user':
        return null;
      case 'plugin:fs|mkdir':
      case 'plugin:window|show':
        return;
      case 'efex_set_usb_backend':
        (qa.backendChanges ??= []).push(args.backend);
        return;
      case 'supervisor_list_runs':
        return qa.runs || [];
      case 'supervisor_list_rounds':
        return qa.rounds || [];
      case 'supervisor_control_run':
        qa.runs = [];
        return;
      case 'supervisor_list_human_requests':
      case 'diagnostics_list_incidents':
      case 'workbench_list_hosts':
      case 'adb_list_devices':
      case 'ch347_list_devices':
        return [];
      default:
        throw Error('Unconfigured test IPC: ' + cmd);
    }
  },
  { shouldMockEvents: true }
);
