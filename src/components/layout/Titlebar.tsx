import { useState, useEffect } from "react";
import { isTauri } from "../../utils/tauri";
import { Minus, Square, Copy, X } from "lucide-react";

interface TitlebarProps {
  breadcrumb?: string;
  onBreadcrumbClick?: () => void;
  activeStatus?: {
    label: string;
    tone: "emerald" | "amber" | "rose";
    onClick?: () => void;
  } | null;
}

export function Titlebar({ breadcrumb, onBreadcrumbClick, activeStatus }: TitlebarProps) {
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
    <header className="h-9 w-full bg-[#08080a] border-b border-white/[0.08] flex items-center justify-between px-3 select-none z-50 shrink-0">
      <div
        data-tauri-drag-region
        className="flex items-center gap-2.5 flex-1 h-full cursor-default overflow-hidden pr-4"
      >
        <div className="flex items-center gap-2 shrink-0">
          <img
            src="/app-logo.png"
            alt="SoundShift"
            className="w-4 h-4 object-contain"
          />
          <span className="font-mono text-xs font-semibold tracking-wider text-zinc-200">
            SOUNDSHIFT
          </span>
          <span className="text-[10px] font-mono text-zinc-500 bg-white/[0.04] px-1.5 py-0.2 rounded border border-white/[0.08]">
            v0.10.0
          </span>
        </div>

        {breadcrumb && (
          <div className="flex items-center gap-1.5 text-xs text-zinc-500 truncate ml-2">
            <span>/</span>
            {onBreadcrumbClick ? (
              <button
                type="button"
                onClick={onBreadcrumbClick}
                className="text-zinc-300 hover:text-zinc-100 font-medium truncate transition-colors cursor-pointer"
              >
                {breadcrumb}
              </button>
            ) : (
              <span className="text-zinc-300 font-medium truncate">{breadcrumb}</span>
            )}
          </div>
        )}
      </div>

      {activeStatus && (
        <button
          type="button"
          onClick={activeStatus.onClick}
          className={`mr-2 hidden sm:inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full border text-[10px] font-mono transition-colors cursor-pointer shrink-0 ${
            activeStatus.tone === "rose"
              ? "bg-rose-950/60 border-rose-500/35 text-rose-300 hover:bg-rose-900/60"
              : activeStatus.tone === "amber"
              ? "bg-amber-950/60 border-amber-500/35 text-amber-300 hover:bg-amber-900/60"
              : "bg-emerald-950/60 border-emerald-500/35 text-emerald-300 hover:bg-emerald-900/60"
          }`}
        >
          <span
            className={`w-1.5 h-1.5 rounded-full ${
              activeStatus.tone === "rose"
                ? "bg-rose-400"
                : activeStatus.tone === "amber"
                ? "bg-amber-400 animate-pulse"
                : "bg-emerald-400 animate-pulse"
            }`}
          />
          <span>{activeStatus.label}</span>
        </button>
      )}

      <div className="flex items-center gap-0.5 shrink-0">
        <button
          type="button"
          onClick={handleMinimize}
          className="p-1 hover:bg-white/[0.08] text-zinc-400 hover:text-zinc-200 rounded transition-colors cursor-pointer"
          title="Minimize"
          aria-label="Minimize"
        >
          <Minus className="w-3.5 h-3.5" />
        </button>
        <button
          type="button"
          onClick={handleToggleMaximize}
          className="p-1 hover:bg-white/[0.08] text-zinc-400 hover:text-zinc-200 rounded transition-colors cursor-pointer"
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
          className="p-1 hover:bg-rose-950/80 hover:text-rose-200 text-zinc-400 rounded transition-colors cursor-pointer"
          title="Close"
          aria-label="Close"
        >
          <X className="w-3.5 h-3.5" />
        </button>
      </div>
    </header>
  );
}
