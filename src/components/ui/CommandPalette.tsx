import React, { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Search, ArrowRight, Keyboard } from "lucide-react";
import { acquireShellInert } from "../../utils/shellInert";

export interface PaletteCommand {
  id: string;
  label: string;
  hint?: string;
  keywords?: string;
  onSelect: () => void;
}

interface CommandPaletteProps {
  isOpen: boolean;
  onClose: () => void;
  commands: PaletteCommand[];
}

export const CommandPalette: React.FC<CommandPaletteProps> = ({
  isOpen,
  onClose,
  commands,
}) => {
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const overlayRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return commands;
    return commands.filter(
      (cmd) =>
        cmd.label.toLowerCase().includes(q) ||
        (cmd.hint?.toLowerCase().includes(q) ?? false) ||
        (cmd.keywords?.toLowerCase().includes(q) ?? false),
    );
  }, [commands, query]);

  useEffect(() => {
    setActiveIndex(0);
  }, [query, isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    previousFocusRef.current = document.activeElement as HTMLElement | null;
    const releaseInert = acquireShellInert(overlayRef.current);

    setQuery("");
    const focusTimer = window.setTimeout(() => {
      inputRef.current?.focus();
    }, 0);

    return () => {
      window.clearTimeout(focusTimer);
      releaseInert();
      previousFocusRef.current?.focus?.();
    };
  }, [isOpen]);

  if (!isOpen) return null;

  const runCommand = (cmd: PaletteCommand | undefined) => {
    if (!cmd) return;
    onClose();
    cmd.onSelect();
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setActiveIndex((idx) => (filtered.length > 0 ? (idx + 1) % filtered.length : 0));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setActiveIndex((idx) =>
        filtered.length > 0 ? (idx - 1 + filtered.length) % filtered.length : 0,
      );
    } else if (e.key === "Enter") {
      e.preventDefault();
      runCommand(filtered[activeIndex]);
    }
  };

  const content = (
    <div
      ref={overlayRef}
      role="dialog"
      aria-modal="true"
      aria-label="Command Palette"
      className="fixed inset-0 z-50 flex items-start justify-center pt-[12vh] p-4 bg-black/75 backdrop-blur-md animate-in fade-in duration-100"
      onClick={onClose}
      onKeyDown={handleKeyDown}
    >
      <div
        className="w-full max-w-lg rounded-2xl border border-white/[0.12] bg-[#0a0a0f] shadow-2xl shadow-black/80 overflow-hidden text-zinc-100"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2.5 px-4 py-3 border-b border-white/[0.08] bg-[#0e0e14]">
          <Search className="w-4 h-4 text-zinc-500 shrink-0" />
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Jump to view or run quick action..."
            aria-label="Command search"
            className="w-full bg-transparent text-xs text-zinc-100 placeholder:text-zinc-500 focus:outline-none"
          />
          <kbd className="hidden sm:inline-flex items-center gap-1 px-1.5 py-0.5 text-[10px] font-mono text-zinc-400 bg-white/[0.05] border border-white/[0.08] rounded">
            ESC
          </kbd>
        </div>

        <ul
          role="listbox"
          aria-label="Command options"
          className="max-h-72 overflow-y-auto p-1.5 space-y-0.5 dialog-overscroll scrollbar-thin scrollbar-thumb-zinc-800"
        >
          {filtered.length === 0 ? (
            <li className="px-3 py-6 text-center text-xs text-zinc-500">
              No matching commands found.
            </li>
          ) : (
            filtered.map((cmd, idx) => {
              const selected = idx === activeIndex;
              return (
                <li key={cmd.id}>
                  <button
                    type="button"
                    role="option"
                    aria-selected={selected}
                    onMouseEnter={() => setActiveIndex(idx)}
                    onClick={() => runCommand(cmd)}
                    className={`w-full flex items-center justify-between gap-3 px-3 py-2 rounded-lg text-left text-xs transition-colors cursor-pointer ${
                      selected
                        ? "bg-emerald-500/15 text-emerald-200 border border-emerald-500/30"
                        : "text-zinc-300 hover:bg-white/[0.05] border border-transparent"
                    }`}
                  >
                    <div className="min-w-0">
                      <div className="font-medium truncate">{cmd.label}</div>
                      {cmd.hint && (
                        <div className="text-[10px] text-zinc-500 truncate mt-0.5">{cmd.hint}</div>
                      )}
                    </div>
                    <ArrowRight
                      className={`w-3.5 h-3.5 shrink-0 ${selected ? "text-emerald-400" : "text-zinc-600"}`}
                    />
                  </button>
                </li>
              );
            })
          )}
        </ul>

        <div className="flex items-center justify-between px-4 py-2 border-t border-white/[0.06] bg-[#08080c] text-[10px] text-zinc-500 font-mono">
          <span className="flex items-center gap-1.5">
            <Keyboard className="w-3 h-3" />
            <span>Up/Down to navigate, Enter to select</span>
          </span>
          <span>Ctrl+K</span>
        </div>
      </div>
    </div>
  );

  return typeof document !== "undefined" ? createPortal(content, document.body) : content;
};
