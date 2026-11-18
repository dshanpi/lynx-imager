import {
  Bot24Regular,
  ClipboardTaskListLtr24Regular,
  DeveloperBoard24Regular,
  Flash24Regular,
  Layer24Regular,
  Settings24Regular,
} from '@fluentui/react-icons';
import { useAppVersion } from '../../Hooks/useAppVersion';

export function V2BasicHeader({
  onAdvanced,
  onAI,
  onRecords,
  onSettings,
}: {
  onAdvanced: () => void;
  onAI: () => void;
  onRecords: () => void;
  onSettings: () => void;
}) {
  const version = useAppVersion();
  return (
    <header className="v2-top" aria-label="应用导航">
      <div className="v2-brand">
        <DeveloperBoard24Regular aria-hidden="true" />
        <strong>LYNX Imager</strong>
        <small>v{version}</small>
      </div>
      <nav aria-label="工作区">
        <button type="button" className="is-current" aria-current="page">
          <Flash24Regular />
          基础烧录
        </button>
        <button type="button" onClick={onAdvanced}>
          <Layer24Regular />
          高级烧录
        </button>
        <button type="button" onClick={onAI}>
          <Bot24Regular />
          连接 AI
        </button>
      </nav>
      <div className="v2-top-tools">
        <button type="button" aria-label="任务记录" onClick={onRecords}>
          <ClipboardTaskListLtr24Regular />
          <span>任务记录</span>
        </button>
        <button type="button" aria-label="设置" onClick={onSettings}>
          <Settings24Regular />
          <span>设置</span>
        </button>
      </div>
    </header>
  );
}
