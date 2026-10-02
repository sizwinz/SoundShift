/**
 * Reliable environment detection for Tauri v2 native desktop runtime.
 * Checks for window.__TAURI_INTERNALS__, window.__TAURI__, and window.isTauri.
 */
export function isTauri(): boolean {
  if (typeof window === 'undefined') {
    return false;
  }

  const win = window as unknown as {
    __TAURI_INTERNALS__?: unknown;
    __TAURI__?: unknown;
    isTauri?: boolean;
  };

  return Boolean(win.__TAURI_INTERNALS__ || win.__TAURI__ || win.isTauri);
}
