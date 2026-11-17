import { useState } from 'react';
import { invokeCommand } from '../../Platform/IPC';
import type { DeviceFlowNode } from './deviceFlowModel';

interface DeviceFlowNodeFieldsProps {
  node: DeviceFlowNode;
  update: (key: string, value: unknown) => void;
}

export const DeviceFlowNodeFields = ({ node, update }: DeviceFlowNodeFieldsProps) => {
  const text = (key: string, label: string, placeholder = '') => (
    <label>
      {label}
      <input
        value={String(node.config[key] ?? '')}
        placeholder={placeholder}
        onChange={(event) => update(key, event.target.value)}
      />
    </label>
  );
  const number = (key: string, label: string, min = 0) => (
    <label>
      {label}
      <input
        type="number"
        min={min}
        value={Number(node.config[key] ?? 0)}
        onChange={(event) => update(key, Number(event.target.value))}
      />
    </label>
  );
  if (node.type.includes('serial_match') || node.type === 'serial.expect')
    return (
      <>
        {text('pattern', '匹配内容', '固定文本')}
        {number('timeoutMs', '超时（毫秒）', 100)}
      </>
    );
  if (node.type === 'serial.send')
    return (
      <>
        {text('text', '发送内容')}
        <label className="ais-check">
          <input
            type="checkbox"
            checked={Boolean(node.config.appendEnter)}
            onChange={(event) => update('appendEnter', event.target.checked)}
          />
          末尾发送回车
        </label>
      </>
    );
  if (node.type === 'serial.login')
    return (
      <>
        {text('username', '用户名')}
        <DeviceFlowCredentialField key={node.id} node={node} update={update} />
        {text('loginPrompt', '登录提示符')}
        {text('passwordPrompt', '密码提示符')}
        {text('prompt', 'Shell 提示符')}
        {number('timeoutMs', '超时（毫秒）', 100)}
      </>
    );
  if (node.type === 'serial.command' || node.type === 'adb.command')
    return (
      <>
        {text('command', '设备命令', '显式配置，不由 AI 任意生成')}
        {node.type === 'serial.command' && text('prompt', '命令前 Shell 提示符')}
        {number('timeoutMs', '超时（毫秒）', 100)}
      </>
    );
  if (node.type === 'power.action')
    return (
      <>
        <label>
          动作
          <select
            value={String(node.config.action ?? 'get_state')}
            onChange={(event) => update('action', event.target.value)}
          >
            <option value="get_state">读取状态</option>
            <option value="power_on">上电</option>
            <option value="power_off">下电</option>
            <option value="reset_pulse">复位脉冲</option>
            <option value="boot_assert">Boot 拉低</option>
            <option value="boot_release">Boot 释放</option>
          </select>
        </label>
        {number('pulseMs', '脉冲时间（毫秒）', 10)}
      </>
    );
  if (node.type === 'control.delay') return number('delayMs', '延时（毫秒）', 10);
  if (node.type === 'control.retry')
    return (
      <>
        {number('attempts', '最多次数', 1)}
        {number('delayMs', '重试间隔（毫秒）', 10)}
      </>
    );
  if (node.type === 'control.condition') return text('expression', '条件表达式');
  if (node.type === 'verify.deterministic')
    return (
      <>
        <label>
          验收方式
          <select
            value={String(node.config.kind ?? 'output_match')}
            onChange={(event) => update('kind', event.target.value)}
          >
            <option value="output_match">输出匹配</option>
            <option value="task_status">任务状态</option>
          </select>
        </label>
        {text('pattern', '期望值')}
      </>
    );
  if (node.type === 'mcu.operation')
    return (
      <label>
        受控 MCU 动作
        <select
          value={String(node.config.action ?? 'connect')}
          onChange={(event) => update('action', event.target.value)}
        >
          <option value="connect">连接调试器</option>
          <option value="reset">复位</option>
          <option value="flash">烧录已登记固件</option>
          <option value="read_register">读取寄存器</option>
        </select>
      </label>
    );
  if (node.type === 'flash.workbench' || node.type === 'ssh.build')
    return <>{number('projectId', '已登记项目 ID', 1)}</>;
  if (node.type === 'trigger.schedule')
    return <>{number('intervalSeconds', '运行间隔（秒）', 10)}</>;
  if (node.type === 'trigger.device')
    return (
      <>
        {number('vendorId', 'USB VID（十进制）', 0)}
        {number('productId', 'USB PID（十进制）', 0)}
        {number('cooldownSeconds', '冷却时间（秒）', 10)}
      </>
    );
  if (node.type.startsWith('trigger.'))
    return <>{number('cooldownSeconds', '冷却时间（秒）', 10)}</>;
  return null;
};

const DeviceFlowCredentialField = ({ node, update }: DeviceFlowNodeFieldsProps) => {
  const [password, setPassword] = useState('');
  const [status, setStatus] = useState('未设置密码时按无密码登录处理');
  const [saving, setSaving] = useState(false);
  const reference =
    String(node.config.credentialRef ?? '').trim() ||
    `serial-login:${String(node.config.boardId ?? 'unbound')}:${node.id}`;

  const savePassword = async () => {
    if (!password) {
      setStatus('请输入密码；无密码登录不需要保存凭据。');
      return;
    }
    setSaving(true);
    try {
      await invokeCommand('supervisor_save_device_flow_credential', { reference, password });
      update('credentialRef', reference);
      setPassword('');
      setStatus('密码已写入系统密钥环，任务中只保存引用。');
    } catch {
      setStatus('密码保存失败；请确认系统密钥环当前可用。');
    } finally {
      setSaving(false);
    }
  };

  const clearPassword = async () => {
    setSaving(true);
    try {
      await invokeCommand('supervisor_save_device_flow_credential', {
        reference,
        password: null,
      });
      update('credentialRef', '');
      setPassword('');
      setStatus('已清除密钥环密码，将按无密码登录处理。');
    } catch {
      setStatus('密钥环中没有可清除的密码，当前按无密码登录处理。');
      update('credentialRef', '');
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="ais-flow-credential">
      <label>
        登录密码（系统密钥环）
        <input
          type="password"
          autoComplete="new-password"
          value={password}
          placeholder="无密码登录请留空"
          onChange={(event) => setPassword(event.target.value)}
        />
      </label>
      <div>
        <button type="button" disabled={saving || !password} onClick={() => void savePassword()}>
          保存密码
        </button>
        <button
          type="button"
          disabled={saving || !String(node.config.credentialRef ?? '')}
          onClick={() => void clearPassword()}
        >
          清除密码
        </button>
      </div>
      <small>{status}</small>
    </div>
  );
};
