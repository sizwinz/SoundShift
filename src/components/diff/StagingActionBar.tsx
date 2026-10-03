import React from "react";
import { CheckCheck, Ban, Copy, ArrowRight } from "lucide-react";

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
  return (
    <div className="sticky bottom-0 left-0 right-0 h-16 bg-[#09090b]/95 backdrop-blur-md border-t border-[#27272a] px-4 sm:px-6 flex items-center justify-between z-20 shadow-2xl">
      {/* Left: Telemetry & Category Dots per D-11 */}
      <div className="flex items-center gap-4 sm:gap-6">
        <div>
          <div className="text-xs font-semibold text-zinc-100 flex items-center gap-2">
            <span>{selectedCount} of {totalCount} tracks selected</span>
          </div>
          <div className="flex items-center gap-3 text-[11px] text-zinc-400 mt-0.5">
            <span className="flex items-center gap-1">
              <span className="w-2 h-2 rounded-full bg-emerald-500" />
              <span>{exactCount} Exact</span>
            </span>
            <span className="flex items-center gap-1">
              <span className="w-2 h-2 rounded-full bg-amber-500" />
              <span>{amberCount} Amber</span>
            </span>
            <span className="flex items-center gap-1">
              <span className="w-2 h-2 rounded-full bg-rose-500" />
              <span>{redCount} Red</span>
            </span>
          </div>
        </div>

        {/* Batch Action Buttons per DIFF-06 and D-12 */}
        <div className="hidden lg:flex items-center gap-2 pl-4 border-l border-zinc-800">
          <button
            type="button"
            onClick={onAcceptAllAmbiguous}
            disabled={amberCount === 0}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-zinc-900 hover:bg-zinc-800 border border-zinc-800 text-xs font-medium text-amber-400 hover:text-amber-300 transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
            title="Promote top algorithmic recommendation for all Amber tracks and mark selected"
          >
            <CheckCheck className="w-3.5 h-3.5" />
            <span>Accept All Ambiguous</span>
          </button>

          <button
            type="button"
            onClick={onSkipUnresolved}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-zinc-900 hover:bg-zinc-800 border border-zinc-800 text-xs font-medium text-zinc-400 hover:text-zinc-200 transition-colors cursor-pointer"
            title="Uncheck all unapproved Amber tracks and all Red tracks"
          >
            <Ban className="w-3.5 h-3.5 text-zinc-500" />
            <span>Skip Unresolved</span>
          </button>

          <button
            type="button"
            onClick={onDeduplicate}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-zinc-900 hover:bg-zinc-800 border border-zinc-800 text-xs font-medium text-zinc-400 hover:text-zinc-200 transition-colors cursor-pointer"
            title="Uncheck duplicate track occurrences while retaining the first occurrence"
          >
            <Copy className="w-3.5 h-3.5 text-zinc-500" />
            <span>Deduplicate</span>
          </button>
        </div>
      </div>

      {/* Right: Transfer Action Button */}
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={onStartTransfer}
          disabled={selectedCount === 0}
          className="flex items-center gap-2 px-5 py-2.5 rounded bg-emerald-500 hover:bg-emerald-400 text-black font-bold text-xs transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed shadow-md"
        >
          <span>Transfer Selected ({selectedCount})</span>
          <ArrowRight className="w-3.5 h-3.5" />
        </button>
      </div>
    </div>
  );
};
