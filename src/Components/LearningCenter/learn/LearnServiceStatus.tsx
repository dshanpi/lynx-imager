import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../../Platform/IPC/Client';
import { learnServiceError, learnServiceMessage } from './serviceStatus';

export function LearnServiceStatus({
  active,
  accountId,
  onRequestLogin,
}: {
  active: boolean;
  accountId: number | null;
  onRequestLogin: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('');
  const generation = useRef(0);
  const pending = useRef(false);
  useEffect(
    () => () => {
      generation.current++;
      pending.current = false;
    },
    []
  );
  const check = async () => {
    if (!active || pending.current) return;
    if (accountId === null) {
      onRequestLogin();
      return;
    }
    const request = ++generation.current;
    pending.current = true;
    setBusy(true);
    setMessage('正在检查官网课程接口…');
    try {
      const result = await invokeCommand('learn_get_capabilities');
      if (request === generation.current) setMessage(learnServiceMessage(result));
    } catch (error) {
      if (request === generation.current) setMessage(learnServiceError(error));
    } finally {
      if (request === generation.current) {
        pending.current = false;
        setBusy(false);
      }
    }
  };
  return (
    <div className="learn-notice" data-testid="learn-service-status">
      <button className="learning-text-button" disabled={busy} onClick={check}>
        {busy ? '检查中…' : '检查官网连接'}
      </button>
      {message && (
        <span role="status" aria-live="polite">
          {message}
        </span>
      )}
    </div>
  );
}
