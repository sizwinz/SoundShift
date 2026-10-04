import React from "react";
import { CheckCheck, Ban, Copy, ArrowRight } from "lucide-react";
import { useTransfer } from "../../context/TransferContext";

interface StagingActionBarProps {
  selectedCount: number;
  totalCount: number;
  exactCount: number;
  amberCount: number;
  redCount: number;
  onAcceptAllAmbiguous: () => void;
  onSkipUnresolved: () => void;
  onDeduplicate: () => void;
  onStartTransfer: () => void;
}

export const StagingActionBar: React.FC<StagingActionBarProps> = ({
  selectedCount,
  totalCount,
  exactCount,
  amberCount,
  redCount,
  onAcceptAllAmbiguous,
  onSkipUnresolved,
  onDeduplicate,
  onStartTransfer,
}) => {
  const { isDrawerOpen, isDrawerExpanded } = useTransfer();
  const selectedPercent = totalCount > 0 ? Math.round((selectedCount / totalCount) * 100) : 0;
  const exactPercent = totalCount > 0 ? (exactCount / totalCount) * 100 : 0;
  const amberPercent = totalCount > 0 ? (amberCount / totalCount) * 100 : 0;
  const redPercent = totalCount > 0 ? (redCount / totalCount) * 100 : 0;

  return (
    <div
      className="sticky z-20 border-t border-[#27272a] bg-[#0b0b0e]/[.98] shadow-[0_-16px_32px_rgba(0,0,0,.28)] backdrop-blur-xl transition-[bottom] duration-300"
      style={{
        bottom: isDrawerOpen
          ? isDrawerExpanded
            ? "min(28rem, 70dvh)"
            : "3.5rem"
          : "0px",
      }}
    >
      <div className="h-1 w-full bg-zinc-900 flex overflow-hidden" aria-label="Match quality distribution">
        <span className="bg-emerald-500 transition-all" style={{ width: `${exactPercent}%` }} />
        <span className="bg-amber-500 transition-all" style={{ width: `${amberPercent}%` }} />
        <span className="bg-rose-500 transition-all" style={{ width: `${redPercent}%` }} />
      </div>

      <div className="px-3 py-3 sm:px-5 lg:px-6">
        <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
          <div className="min-w-0 flex-1">
            <div className="flex items-baseline justify-between gap-3">
              <div className="text-sm font-semibold text-zinc-100">
                {selectedCount} <span className="font-normal text-zinc-500">of {totalCount} selected</span>
              </div>
              <span className="text-xs font-mono text-emerald-400">{selectedPercent}% ready</span>
            </div>
            <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-zinc-800">
              <div
                className="h-full rounded-full bg-emerald-500 transition-all duration-300"
                style={{ width: `${selectedPercent}%` }}
              />
            </div>
            <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] text-zinc-400">
              <span><strong className="text-emerald-400">{exactCount}</strong> exact</span>
              <span><strong className="text-amber-400">{amberCount}</strong> needs review</span>
              <span><strong className="text-rose-400">{redCount}</strong> unmatched</span>
            </div>
          </div>

          <div className="flex flex-wrap items-center gap-2 lg:justify-end">
            {/* Batch actions stay beside the primary action on desktop and wrap cleanly on narrow windows. */}
            <button
              type="button"
              onClick={onAcceptAllAmbiguous}
              disabled={amberCount === 0}
              className="inline-flex h-9 items-center gap-1.5 rounded-md border border-amber-500/20 bg-amber-500/5 px-3 text-xs font-medium text-amber-300 transition hover:bg-amber-500/10 disabled:cursor-not-allowed disabled:opacity-35"
              title="Promote safe ambiguous recommendations"
            >
              <CheckCheck className="h-3.5 w-3.5" />
              <span className="hidden sm:inline">Accept reviewable</span>
              <span className="sm:hidden">Accept</span>
            </button>

            <button
              type="button"
              onClick={onSkipUnresolved}
              className="inline-flex h-9 items-center gap-1.5 rounded-md border border-zinc-800 bg-zinc-900 px-3 text-xs font-medium text-zinc-300 transition hover:border-zinc-700 hover:bg-zinc-800"
              title="Uncheck all unresolved tracks"
            >
              <Ban className="h-3.5 w-3.5 text-zinc-500" />
              <span className="hidden sm:inline">Skip unresolved</span>
              <span className="sm:hidden">Skip</span>
            </button>

            <button
              type="button"
              onClick={onDeduplicate}
              className="inline-flex h-9 items-center gap-1.5 rounded-md border border-zinc-800 bg-zinc-900 px-3 text-xs font-medium text-zinc-300 transition hover:border-zinc-700 hover:bg-zinc-800"
              title="Uncheck duplicate occurrences"
            >
              <Copy className="h-3.5 w-3.5 text-zinc-500" />
              <span className="hidden sm:inline">Deduplicate</span>
              <span className="sm:hidden">Dedupe</span>
            </button>

            <button
              type="button"
              onClick={onStartTransfer}
              disabled={selectedCount === 0}
              className="inline-flex h-10 min-w-[190px] items-center justify-center gap-2 rounded-md bg-emerald-500 px-4 text-xs font-bold text-zinc-950 shadow-[0_8px_24px_rgba(16,185,129,.16)] transition hover:-translate-y-px hover:bg-emerald-400 active:translate-y-0 disabled:cursor-not-allowed disabled:opacity-35"
            >
              <span>Transfer selected</span>
              <span className="rounded bg-black/10 px-1.5 py-0.5 font-mono">({selectedCount})</span>
              <ArrowRight className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};
