import { useState, useEffect } from "react";
import { isTauri } from "../../utils/tauri";
import { Minus, Square, Copy, X } from "lucide-react";

export function Titlebar() {
  const [isMaximized, setIsMaximized] = useState(false);
  const inTauri = isTauri();

  useEffect(() => {
    if (!inTauri) return;

    let unlisten: (() => void) | undefined;

    const setupWindow = async () => {
      try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        const appWindow = getCurrentWindow();
        const maximized = await appWindow.isMaximized();
        setIsMaximized(maximized);

        unlisten = await appWindow.onResized(async () => {
          try {
            const currentMaximized = await appWindow.isMaximized();
            setIsMaximized(currentMaximized);
          } catch {
            // Ignore during rapid resize
          }
        });
      } catch (err) {
        console.warn("Failed to initialize window controls:", err);
      }
    };

    setupWindow();

    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  }, [inTauri]);

  const handleMinimize = async () => {
    if (!inTauri) return;
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().minimize();
    } catch (err) {
      console.warn("Minimize error:", err);
    }
  };

  const handleToggleMaximize = async () => {
    if (!inTauri) return;
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const appWindow = getCurrentWindow();
      await appWindow.toggleMaximize();
      const maximized = await appWindow.isMaximized();
      setIsMaximized(maximized);
    } catch (err) {
      console.warn("Toggle maximize error:", err);
    }
  };

  const handleClose = async () => {
    if (!inTauri) return;
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      await getCurrentWindow().close();
    } catch (err) {
      console.warn("Close error:", err);
    }
  };

  return (
    <header className="h-9 w-full bg-[#09090b] border-b border-[#27272a] flex items-center justify-between px-3 select-none z-50">
      <div
        data-tauri-drag-region
        className="flex items-center gap-2.5 flex-1 h-full cursor-default"
      >
        <img
          src="/app-logo.png"
          alt="SoundShift"
          className="w-4 h-4 object-contain"
        />
        <span className="font-mono text-xs font-bold tracking-widest text-zinc-200">
          SOUNDSHIFT
        </span>
        <span className="text-[10px] font-mono text-zinc-500 bg-[#121215] px-1.5 py-0.5 rounded border border-[#27272a]">
          v0.9.0-beta
        </span>
      </div>

      <div className="flex items-center gap-1">
        <button
          type="button"
          onClick={handleMinimize}
          className="p-1 hover:bg-[#18181b] text-zinc-400 hover:text-zinc-200 rounded transition-colors cursor-pointer"
          title="Minimize"
          aria-label="Minimize"
        >
          <Minus className="w-3.5 h-3.5" />
        </button>
        <button
          type="button"
          onClick={handleToggleMaximize}
          className="p-1 hover:bg-[#18181b] text-zinc-400 hover:text-zinc-200 rounded transition-colors cursor-pointer"
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
          className="p-1 hover:bg-[#7f1d1d] text-zinc-400 hover:text-red-200 rounded transition-colors cursor-pointer"
          title="Close"
          aria-label="Close"
        >
          <X className="w-3.5 h-3.5" />
        </button>
      </div>
    </header>
  );
}
