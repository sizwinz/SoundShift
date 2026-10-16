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
  const { stage } = useTransfer();
  const isTransferring = stage === "starting" || stage === "transferring" || stage === "paused";
  const selectedPercent = totalCount > 0 ? Math.round((selectedCount / totalCount) * 100) : 0;
  const exactPercent = totalCount > 0 ? (exactCount / totalCount) * 100 : 0;
  const amberPercent = totalCount > 0 ? (amberCount / totalCount) * 100 : 0;
  const redPercent = totalCount > 0 ? (redCount / totalCount) * 100 : 0;

  return (
    <div className="shrink-0 border-t border-white/[0.08] bg-[#09090d]/95 shadow-[0_-8px_32px_rgba(0,0,0,0.5)] backdrop-blur-xl">
      {/* Quality distribution bar */}
      <div className="h-1 w-full bg-white/[0.04] flex overflow-hidden" aria-label="Match quality distribution">
        <span className="bg-emerald-500 transition-all duration-300" style={{ width: `${exactPercent}%` }} />
        <span className="bg-amber-500 transition-all duration-300" style={{ width: `${amberPercent}%` }} />
        <span className="bg-rose-500 transition-all duration-300" style={{ width: `${redPercent}%` }} />
      </div>

      <div className="w-full max-w-[1440px] mx-auto px-4 py-3 sm:px-5 lg:px-6">
        <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
          {/* Left Summary & Progress */}
          <div className="min-w-0 flex-1">
            <div className="flex items-baseline justify-between gap-3">
              <div className="text-sm font-semibold text-zinc-100 tabular-nums">
                {selectedCount} <span className="font-normal text-zinc-500 text-xs">of {totalCount} selected</span>
              </div>
              <span className="text-xs font-mono font-medium text-emerald-400 tabular-nums">{selectedPercent}% ready</span>
            </div>

            <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-white/[0.06]">
              <div
                className="h-full rounded-full bg-emerald-500 transition-all duration-300 shadow-[0_0_8px_rgba(16,185,129,0.4)]"
                style={{ width: `${selectedPercent}%` }}
              />
            </div>

            <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-[11px] text-zinc-400 font-mono">
              <span><strong className="text-emerald-400 font-semibold">{exactCount}</strong> exact</span>
              <span><strong className="text-amber-400 font-semibold">{amberCount}</strong> needs review</span>
              <span><strong className="text-rose-400 font-semibold">{redCount}</strong> unmatched</span>
            </div>
          </div>

          {/* Action Buttons Deck */}
          <div className="flex flex-wrap items-center gap-2 lg:justify-end">
            <button
              type="button"
              onClick={onAcceptAllAmbiguous}
              disabled={amberCount === 0 || isTransferring}
              className="inline-flex h-9 items-center gap-1.5 rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 text-xs font-medium text-amber-300 transition-all hover:bg-amber-500/20 active:translate-y-px cursor-pointer disabled:cursor-not-allowed disabled:opacity-35"
              title="Promote safe ambiguous recommendations"
            >
              <CheckCheck className="h-3.5 w-3.5" />
              <span className="hidden sm:inline">Accept reviewable</span>
              <span className="sm:hidden">Accept</span>
            </button>

            <button
              type="button"
              onClick={onSkipUnresolved}
              disabled={isTransferring}
              className="inline-flex h-9 items-center gap-1.5 rounded-lg border border-white/[0.08] bg-[#121217] px-3 text-xs font-medium text-zinc-300 transition-all hover:bg-[#181820] hover:border-white/[0.16] active:translate-y-px cursor-pointer disabled:cursor-not-allowed disabled:opacity-35"
              title="Uncheck all unresolved tracks"
            >
              <Ban className="h-3.5 w-3.5 text-zinc-500" />
              <span className="hidden sm:inline">Skip unresolved</span>
              <span className="sm:hidden">Skip</span>
            </button>

            <button
              type="button"
              onClick={onDeduplicate}
              disabled={isTransferring}
              className="inline-flex h-9 items-center gap-1.5 rounded-lg border border-white/[0.08] bg-[#121217] px-3 text-xs font-medium text-zinc-300 transition-all hover:bg-[#181820] hover:border-white/[0.16] active:translate-y-px cursor-pointer disabled:cursor-not-allowed disabled:opacity-35"
              title="Uncheck duplicate occurrences"
            >
              <Copy className="h-3.5 w-3.5 text-zinc-500" />
              <span className="hidden sm:inline">Deduplicate</span>
              <span className="sm:hidden">Dedupe</span>
            </button>

            <button
              type="button"
              onClick={onStartTransfer}
              disabled={selectedCount === 0 || isTransferring}
              className="inline-flex h-9 min-w-[190px] items-center justify-center gap-2 rounded-lg bg-emerald-500 hover:bg-emerald-400 px-4 text-xs font-bold text-zinc-950 shadow-[0_2px_14px_rgba(16,185,129,0.3)] transition-all hover:-translate-y-px active:translate-y-0 cursor-pointer disabled:cursor-not-allowed disabled:opacity-35"
            >
              <span>{isTransferring ? "Transfer in progress..." : "Transfer selected"}</span>
              <span className="rounded bg-black/15 px-1.5 py-0.5 font-mono text-[11px] tabular-nums">({selectedCount})</span>
              <ArrowRight className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};
