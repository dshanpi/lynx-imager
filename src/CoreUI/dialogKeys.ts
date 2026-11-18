import type { KeyboardEvent } from 'react';

export function handleDialogKey(event: KeyboardEvent<HTMLElement>, close: () => void) {
  if (event.defaultPrevented) return;
  if (event.key === 'Escape') {
    event.preventDefault();
    close();
    return;
  }
  if (event.key !== 'Tab') return;
  const controls = Array.from(
    event.currentTarget.querySelectorAll<HTMLElement>(
      'button:enabled, input:enabled, select:enabled, textarea:enabled, a[href], [tabindex="0"]'
    )
  ).filter((element) => element.getClientRects().length > 0);
  const first = controls[0];
  const last = controls[controls.length - 1];
  const edge = event.shiftKey ? first : last;
  if (edge && document.activeElement === edge) {
    event.preventDefault();
    (event.shiftKey ? last : first).focus();
  }
}
