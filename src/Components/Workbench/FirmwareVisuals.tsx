import { LynxIcon } from '../../CoreUI';
import type { LynxIconName } from '../../CoreUI/LynxIcon/LynxIcon';
export function FirmwareAppIcon({ id }: { id: string }) {
  if (id === 'nekobox')
    return (
      <svg viewBox="0 0 48 48" aria-hidden="true">
        <path
          fill="currentColor"
          d="M9 16 8 7l12 7a25 25 0 0 1 8 0L40 7l-1 10c4 5 5 11 2 17-6 10-28 10-34 0-3-6-2-12 2-18Z"
        />
        <circle cx="17" cy="26" r="2.4" fill="#8056dc" />
        <circle cx="31" cy="26" r="2.4" fill="#8056dc" />
        <path
          d="m21 33 3 2 3-2"
          fill="none"
          stroke="#8056dc"
          strokeWidth="2.4"
          strokeLinecap="round"
        />
      </svg>
    );
  const names: Record<string, LynxIconName> = {
    'usb-display': 'desktop',
    micropython: 'code',
    lvgl: 'learning',
    'hello-world': 'terminal',
    shell: 'terminal',
  };
  return <LynxIcon name={names[id] || 'box'} size={40} />;
}
export function NekoBoxIllustration({ waiting = false }: { waiting?: boolean }) {
  return (
    <div
      className={`fw-neko-illustration ${waiting ? 'is-waiting' : ''}`}
      aria-label="NekoBox 界面示意，非设备实拍"
    >
      <div className="fw-neko-display">
        <strong>NekoBox</strong>
        {waiting ? (
          <div className="fw-neko-wait">
            <FirmwareAppIcon id="nekobox" />
            <span>等待确认实际画面</span>
          </div>
        ) : (
          <div className="fw-neko-apps">
            {(['folder', 'desktop', 'play', 'settings'] as LynxIconName[]).map((name, i) => (
              <div key={name}>
                <LynxIcon name={name} size={24} />
                <small>{['Files', 'Photos', 'Games', 'Settings'][i]}</small>
              </div>
            ))}
          </div>
        )}
      </div>
      <small>界面示意 · 非设备实拍</small>
    </div>
  );
}
