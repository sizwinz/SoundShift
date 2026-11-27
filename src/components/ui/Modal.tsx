import React, { useEffect, useId, useRef } from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";
import { acquireShellInert } from "../../utils/shellInert";

interface ModalProps {
  isOpen: boolean;
  onClose: () => void;
  title: string;
  description?: string;
  children: React.ReactNode;
  footer?: React.ReactNode;
  icon?: React.ReactNode;
  maxWidth?: string;
  disableDismiss?: boolean;
  accent?: "default" | "danger" | "emerald";
  titleId?: string;
}

export const Modal: React.FC<ModalProps> = ({
  isOpen,
  onClose,
  title,
  description,
  children,
  footer,
  icon,
  maxWidth = "max-w-lg",
  disableDismiss = false,
  accent = "default",
  titleId,
}) => {
  const generatedId = useId();
  const resolvedTitleId = titleId ?? generatedId;
  const overlayRef = useRef<HTMLDivElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const previousFocusRef = useRef<HTMLElement | null>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    if (!isOpen) return;

    previousFocusRef.current = document.activeElement as HTMLElement | null;
    const releaseInert = acquireShellInert(overlayRef.current);

    const focusTimer = window.setTimeout(() => {
      panelRef.current?.focus();
    }, 0);

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !disableDismiss) {
        onCloseRef.current();
      }
    };
    window.addEventListener("keydown", handleKeyDown);

    return () => {
      window.clearTimeout(focusTimer);
      window.removeEventListener("keydown", handleKeyDown);
      releaseInert();
      previousFocusRef.current?.focus?.();
    };
  }, [isOpen, disableDismiss]);

  if (!isOpen) return null;

  const borderAccent =
    accent === "danger"
      ? "border-rose-500/35"
      : accent === "emerald"
      ? "border-emerald-500/30"
      : "border-white/[0.1]";

  const content = (
    <div
      ref={overlayRef}
      role="dialog"
      aria-modal="true"
      aria-labelledby={resolvedTitleId}
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/75 backdrop-blur-md animate-in fade-in duration-150"
      onClick={() => {
        if (!disableDismiss) onClose();
      }}
    >
      <div
        ref={panelRef}
        tabIndex={-1}
        className={`w-full ${maxWidth} bg-[#09090d] border ${borderAccent} rounded-2xl shadow-2xl shadow-black/80 overflow-hidden text-zinc-100 max-h-[88vh] flex flex-col focus:outline-none animate-in zoom-in-95 duration-150`}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-start justify-between gap-3 px-5 py-4 border-b border-white/[0.08] bg-[#0c0c11] shrink-0">
          <div className="flex items-start gap-2.5 min-w-0">
            {icon && <div className="shrink-0 mt-0.5">{icon}</div>}
            <div className="min-w-0">
              <h2 id={resolvedTitleId} className="text-sm font-semibold text-zinc-100">
                {title}
              </h2>
              {description && (
                <p className="mt-0.5 text-xs text-zinc-400">{description}</p>
              )}
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={disableDismiss}
            className="p-1 rounded-md text-zinc-400 hover:text-zinc-100 hover:bg-white/[0.08] transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed shrink-0"
            aria-label="Close modal"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        <div className="p-5 overflow-y-auto flex-1 dialog-overscroll scrollbar-thin scrollbar-thumb-zinc-800 scrollbar-track-transparent">
          {children}
        </div>

        {footer && (
          <div className="flex items-center justify-end gap-2.5 px-5 py-3.5 border-t border-white/[0.08] bg-[#0c0c11] shrink-0">
            {footer}
          </div>
        )}
      </div>
    </div>
  );

  return typeof document !== "undefined" ? createPortal(content, document.body) : content;
};
