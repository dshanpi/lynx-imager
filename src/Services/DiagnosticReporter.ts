import { invokeCommand } from '../Platform/IPC';
import type { DiagnosticCaptureRequest } from '../Platform/IPC/Commands';

let installed = false;

function errorDetails(value: unknown): Pick<DiagnosticCaptureRequest, 'messageTemplate' | 'stack'> {
  if (value instanceof Error) {
    return {
      messageTemplate: value.message || value.name || 'Unhandled frontend error',
      stack: value.stack || null,
    };
  }
  if (typeof value === 'string') {
    return { messageTemplate: value, stack: null };
  }
  return { messageTemplate: 'Unhandled non-Error frontend rejection', stack: null };
}

function capture(request: DiagnosticCaptureRequest): void {
  void invokeCommand('diagnostics_capture', { request }).catch(() => undefined);
}

export function installDiagnosticReporter(): () => void {
  if (installed) return () => undefined;
  installed = true;

  const onError = (event: ErrorEvent) => {
    capture({
      source: 'frontend',
      component: 'webview',
      severity: 'error',
      errorCode: 'WEBVIEW_UNHANDLED_ERROR',
      ...errorDetails(event.error ?? event.message),
      operationType: 'window_error',
    });
  };
  const onUnhandledRejection = (event: PromiseRejectionEvent) => {
    capture({
      source: 'frontend',
      component: 'webview',
      severity: 'error',
      errorCode: 'WEBVIEW_UNHANDLED_REJECTION',
      ...errorDetails(event.reason),
      operationType: 'unhandled_rejection',
    });
  };

  window.addEventListener('error', onError);
  window.addEventListener('unhandledrejection', onUnhandledRejection);
  return () => {
    window.removeEventListener('error', onError);
    window.removeEventListener('unhandledrejection', onUnhandledRejection);
    installed = false;
  };
}
