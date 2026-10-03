import React from "react";
import { MatchResult } from "../../types/diff";
import { CheckCircle2, ShieldCheck, ArrowRight, X, Music } from "lucide-react";

interface TransferConfirmModalProps {
  isOpen: boolean;
  onClose: () => void;
  onConfirm: () => void;
  playlistTitle?: string;
  targetService: string;
  results: MatchResult[];
  selectedTrackIds: Set<string>;
}

export const TransferConfirmModal: React.FC<TransferConfirmModalProps> = ({
  isOpen,
  onClose,
  onConfirm,
  playlistTitle = "Playlist Transfer",
  targetService,
  results,
  selectedTrackIds,
}) => {
  if (!isOpen) return null;

  const totalSelected = selectedTrackIds.size;
  const totalTracks = results.length;

  // Compute breakdown
  let exactSelected = 0;
  let manualResolved = 0;

  results.forEach((r) => {
    if (selectedTrackIds.has(r.source_track.id)) {
      if (r.match_method === "manual_disambiguation") {
        manualResolved++;
      } else {
        exactSelected++;
      }
    }
  });

  const skippedCount = totalTracks - totalSelected;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/75 backdrop-blur-xs animate-in fade-in duration-150"
      role="dialog"
      aria-modal="true"
      aria-labelledby="confirm-modal-title"
    >
      <div className="w-full max-w-md bg-[#09090b] border border-[#27272a] rounded-xl shadow-2xl overflow-hidden animate-in zoom-in-95 duration-150">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-[#27272a] bg-[#0c0c0e]">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-lg bg-emerald-950/60 border border-emerald-500/30 flex items-center justify-center text-emerald-400">
              <CheckCircle2 className="w-4 h-4" />
            </div>
            <div>
              <h2 id="confirm-modal-title" className="text-sm font-bold text-zinc-100">
                Confirm Playlist Transfer
              </h2>
              <p className="text-[11px] text-zinc-500">
                Review your staging manifest before executing migration.
              </p>
            </div>
          </div>

          <button
            type="button"
            onClick={onClose}
            className="p-1 rounded-md text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors cursor-pointer"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Body Content */}
        <div className="p-5 space-y-4">
          {/* Target Playlist Summary Card */}
          <div className="p-3.5 rounded-lg bg-[#121215] border border-zinc-800 flex items-center gap-3">
            <div className="w-10 h-10 rounded bg-zinc-800 flex items-center justify-center text-zinc-400 flex-shrink-0">
              <Music className="w-5 h-5" />
            </div>

            <div className="min-w-0 flex-1">
              <div className="text-xs font-semibold text-zinc-200 truncate">
                {playlistTitle}
              </div>
              <div className="text-[11px] text-zinc-400 flex items-center gap-2 mt-0.5">
                <span className="capitalize">Destination: {targetService}</span>
                <span>•</span>
                <span className="text-emerald-400 font-medium font-mono">
                  {totalSelected} tracks to transfer
                </span>
              </div>
            </div>
          </div>

          {/* Breakdown Stats */}
          <div className="grid grid-cols-3 gap-2 text-center">
            <div className="p-2.5 rounded-lg bg-[#0c0c0e] border border-zinc-800">
              <div className="text-base font-bold text-emerald-400 font-mono">
                {exactSelected}
              </div>
              <div className="text-[10px] text-zinc-500 uppercase mt-0.5">Exact Matches</div>
            </div>

            <div className="p-2.5 rounded-lg bg-[#0c0c0e] border border-zinc-800">
              <div className="text-base font-bold text-amber-400 font-mono">
                {manualResolved}
              </div>
              <div className="text-[10px] text-zinc-500 uppercase mt-0.5">Resolved</div>
            </div>

            <div className="p-2.5 rounded-lg bg-[#0c0c0e] border border-zinc-800">
              <div className="text-base font-bold text-zinc-400 font-mono">
                {skippedCount}
              </div>
              <div className="text-[10px] text-zinc-500 uppercase mt-0.5">Skipped</div>
            </div>
          </div>

          {/* Safety Notice per D-13 */}
          <div className="p-3 rounded-lg bg-emerald-950/30 border border-emerald-500/20 text-emerald-300 text-xs flex items-start gap-2.5">
            <ShieldCheck className="w-4 h-4 text-emerald-400 flex-shrink-0 mt-0.5" />
            <div className="text-[11px] leading-relaxed">
              <span className="font-semibold text-emerald-300">1-Click Snapshot Rollback: </span>
              SoundShift will create an immutable local snapshot of your target playlist prior to mutations, allowing instant 1-click rollback at any time.
            </div>
          </div>
        </div>

        {/* Modal Action Buttons */}
        <div className="flex items-center justify-end gap-2.5 px-5 py-3.5 border-t border-[#27272a] bg-[#0c0c0e]">
          <button
            type="button"
            onClick={onClose}
            className="px-3.5 py-1.5 rounded text-xs font-medium text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800 transition-colors cursor-pointer"
          >
            Back to Review
          </button>

          <button
            type="button"
            onClick={onConfirm}
            className="flex items-center gap-1.5 px-4 py-2 rounded bg-emerald-500 hover:bg-emerald-400 text-black font-semibold text-xs transition-colors cursor-pointer shadow-md"
          >
            <span>Confirm & Start Transfer</span>
            <ArrowRight className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>
    </div>
  );
};
