import { useState, useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, Copy, X } from "lucide-react";

export function Titlebar() {
  const [isMaximized, setIsMaximized] = useState(false);
  const appWindow = getCurrentWindow();

  useEffect(() => {
    const checkMaximized = async () => {
      try {
        const maximized = await appWindow.isMaximized();
        setIsMaximized(maximized);
      } catch {
        // Fallback for non-tauri or preview environments
      }
    };

    checkMaximized();

    const unlistenPromise = appWindow.onResized(async () => {
      try {
        const maximized = await appWindow.isMaximized();
        setIsMaximized(maximized);
      } catch {
        // Ignore in browser mock
      }
    });

    return () => {
      unlistenPromise.then((unlisten) => unlisten && unlisten());
    };
  }, [appWindow]);

  const handleMinimize = async () => {
    try {
      await appWindow.minimize();
    } catch {
      // Ignore in mock
    }
  };

  const handleToggleMaximize = async () => {
    try {
      await appWindow.toggleMaximize();
      const maximized = await appWindow.isMaximized();
      setIsMaximized(maximized);
    } catch {
      // Ignore in mock
    }
  };

  const handleClose = async () => {
    try {
      await appWindow.close();
    } catch {
      // Ignore in mock
    }
  };

  return (
    <header className="h-9 w-full bg-[#09090b] border-b border-[#27272a] flex items-center justify-between px-3 select-none z-50">
      <div
        data-tauri-drag-region
        className="flex items-center gap-2.5 flex-1 h-full cursor-default"
      >
        <span className="font-mono text-xs font-bold tracking-widest text-zinc-200">
          SOUNDSHIFT
        </span>
        <span className="text-[10px] font-mono text-zinc-500 bg-[#121215] px-1.5 py-0.5 rounded border border-[#27272a]">
          v0.1.0-alpha
        </span>
      </div>

      <div className="flex items-center gap-1">
        <button
          type="button"
          onClick={handleMinimize}
          className="p-1 hover:bg-[#18181b] text-zinc-400 hover:text-zinc-200 rounded transition-colors"
          title="Minimize"
          aria-label="Minimize"
        >
          <Minus className="w-3.5 h-3.5" />
        </button>
        <button
          type="button"
          onClick={handleToggleMaximize}
          className="p-1 hover:bg-[#18181b] text-zinc-400 hover:text-zinc-200 rounded transition-colors"
          title={isMaximized ? "Restore" : "Maximize"}
          aria-label={isMaximized ? "Restore" : "Maximize"}
        >
          {isMaximized ? (
            <Copy className="w-3.5 h-3.5" />
          ) : (
            <Square className="w-3.5 h-3.5" />
          )}
        </button>
        <button
          type="button"
          onClick={handleClose}
          className="p-1 hover:bg-[#7f1d1d] text-zinc-400 hover:text-red-200 rounded transition-colors"
          title="Close"
          aria-label="Close"
        >
          <X className="w-3.5 h-3.5" />
        </button>
      </div>
    </header>
  );
}
