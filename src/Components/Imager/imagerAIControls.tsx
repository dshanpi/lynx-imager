import React, { useRef, useState } from 'react';
import { formatErrorMessage } from '../../Utils';

export const timeLabel = (ms: number) => new Date(ms).toLocaleString('zh-CN', { hour12: false });

export async function copyVerifiedText(text: Promise<string>) {
  // WebKit requires clipboard.write to start during the click gesture. Supply
  // its data as a promise so a live endpoint can still be checked first.
  if (typeof ClipboardItem !== 'undefined' && navigator.clipboard?.write) {
    const data = text.then((value) => new Blob([value], { type: 'text/plain' }));
    // A webview can reject the clipboard permission before it reads the item.
    void data.catch(() => undefined);
    try {
      const item = new ClipboardItem({ 'text/plain': data });
      await navigator.clipboard.write([item]);
      return;
    } catch {
      /* Try the native webview fallback below. */
    }
  }
  const value = await text;
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(value);
      return;
    } catch {
      /* Older WebKit permissions. */
    }
  }
  const field = document.createElement('textarea');
  field.value = value;
  field.readOnly = true;
  field.style.cssText = 'position:fixed;left:-10000px;top:0';
  document.body.appendChild(field);
  const previous = document.activeElement;
  try {
    field.select();
    if (!document.execCommand('copy')) throw new Error('剪贴板不可用，请选中地址或配置后复制。');
  } finally {
    field.remove();
    if (previous instanceof HTMLElement) previous.focus({ preventScroll: true });
  }
}

export function safeDetail(error: unknown, secrets: readonly string[] = []) {
  const message = formatErrorMessage(error);
  if (
    secrets.some((value) => value && message.includes(value)) ||
    /authorization|bearer\s|(?:api.?key|password|passphrase|token|secret)["']?\s*[:=]|sk-[\w-]{8,}|https?:\/\/[^\s/]+@/i.test(
      message
    )
  ) {
    return '服务返回了含凭据信息的错误。请核对连接与认证设置。';
  }
  return message.slice(0, 400);
}

export function useAIAction(secrets: readonly string[] = []) {
  const lock = useRef(false);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState('');
  async function run(action: () => Promise<string | void>) {
    if (lock.current) return;
    lock.current = true;
    setBusy(true);
    setNotice('');
    try {
      setNotice((await action()) || '');
    } catch (error) {
      setNotice(safeDetail(error, secrets));
    } finally {
      lock.current = false;
      setBusy(false);
    }
  }
  return { run, busy, notice };
}

export function AITable({
  columns,
  children,
  empty,
}: {
  columns: string[];
  children?: React.ReactNode;
  empty?: string;
}) {
  return (
    <div className="im-ai-table-wrap">
      <table>
        <thead>
          <tr>
            {columns.map((column) => (
              <th scope="col" key={column}>
                {column}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {empty ? (
            <tr>
              <td className="im-ai-muted" colSpan={columns.length}>
                {empty}
              </td>
            </tr>
          ) : (
            children
          )}
        </tbody>
      </table>
    </div>
  );
}

export function AIFacts({
  rows,
  columns = ['项目', '当前值'],
}: {
  rows: [string, React.ReactNode][];
  columns?: string[];
}) {
  return (
    <AITable columns={columns}>
      {rows.map(([label, value]) => (
        <tr key={label}>
          <th scope="row">{label}</th>
          <td>{value}</td>
        </tr>
      ))}
    </AITable>
  );
}

export function AIMessage({ children }: { children: React.ReactNode }) {
  return (
    <div className="im-ai-message" role="status" aria-live="polite">
      {children}
    </div>
  );
}
