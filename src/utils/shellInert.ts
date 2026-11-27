type InertElement = HTMLElement & { inert: boolean };

const LOCK_COUNT_ATTR = "data-shell-inert-locks";

/**
 * Marks `#app-shell` inert while an external overlay/dialog is mounted outside it.
 * Uses reference counting so overlapping overlays do not re-enable background clicks prematurely.
 */
export function acquireShellInert(overlayEl?: HTMLElement | null): () => void {
  if (typeof document === "undefined") {
    return () => {};
  }

  const shell = document.getElementById("app-shell") as InertElement | null;
  if (!shell || (overlayEl && shell.contains(overlayEl))) {
    return () => {};
  }

  const currentLocks = Number(shell.getAttribute(LOCK_COUNT_ATTR) || "0");
  shell.setAttribute(LOCK_COUNT_ATTR, String(currentLocks + 1));
  shell.inert = true;

  let released = false;
  return () => {
    if (released) return;
    released = true;

    const remaining = Math.max(0, Number(shell.getAttribute(LOCK_COUNT_ATTR) || "0") - 1);
    if (remaining === 0) {
      shell.removeAttribute(LOCK_COUNT_ATTR);
      shell.inert = false;
    } else {
      shell.setAttribute(LOCK_COUNT_ATTR, String(remaining));
    }
  };
}
