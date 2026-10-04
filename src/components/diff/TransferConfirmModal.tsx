import React, { useState } from "react";
import { MatchResult } from "../../types/diff";
import {
  CheckCircle2,
  ShieldCheck,
  ArrowRight,
  X,
  Plus,
  FolderInput,
  Cpu,
} from "lucide-react";
import { defaultSettings, readSoundShiftSettings } from "../settings/SettingsPage";

export interface TransferTargetOptions {
  playlistName: string;
  isNewPlaylist: boolean;
  targetPlaylistId?: string | null;
  concurrency: number;
}

interface TransferConfirmModalProps {
  isOpen: boolean;
  onClose: () => void;
  onConfirm: (options: TransferTargetOptions) => void;
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
  const [isNewPlaylist, setIsNewPlaylist] = useState(true);
  const [customTitle, setCustomTitle] = useState(playlistTitle);
  const [targetPlaylistId, setTargetPlaylistId] = useState("");
  const [concurrency, setConcurrency] = useState(() => {
    try {
      return readSoundShiftSettings().concurrency;
    } catch {
      return defaultSettings.concurrency;
    }
  });

  if (!isOpen) return null;

  const totalSelected = selectedTrackIds.size;
  const totalTracks = results.length;

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

  const handleConfirm = () => {
    onConfirm({
      playlistName: isNewPlaylist ? customTitle || playlistTitle : playlistTitle,
      isNewPlaylist,
      targetPlaylistId: isNewPlaylist ? null : targetPlaylistId.trim() || null,
      concurrency,
    });
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/80 backdrop-blur-xs animate-in fade-in duration-150"
      role="dialog"
      aria-modal="true"
      aria-labelledby="confirm-modal-title"
    >
      <div className="w-full max-w-lg bg-[#09090b] border border-[#27272a] rounded-xl shadow-2xl overflow-hidden animate-in zoom-in-95 duration-150 max-h-[90vh] flex flex-col">
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
                Configure destination playlist and concurrency before running batch migration.
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
        <div className="p-5 space-y-4 overflow-y-auto flex-1">
          {/* Target Playlist Mode Selector per D-01 */}
          <div className="space-y-2">
            <label className="text-xs font-semibold text-zinc-300">
              Destination Target
            </label>
            <div className="grid grid-cols-2 gap-2">
              <button
                type="button"
                onClick={() => setIsNewPlaylist(true)}
                className={`p-3 rounded-lg border text-left flex flex-col gap-1 transition-colors cursor-pointer ${
                  isNewPlaylist
                    ? "bg-emerald-950/20 border-emerald-500/50 text-emerald-300"
                    : "bg-[#121215] border-zinc-800 text-zinc-400 hover:border-zinc-700"
                }`}
              >
                <div className="flex items-center gap-2 font-medium text-xs">
                  <Plus className="w-3.5 h-3.5 text-emerald-400" />
                  <span>Create New Playlist</span>
                </div>
                <div className="text-[10px] text-zinc-500">
                  Creates a fresh playlist on {targetService}
                </div>
              </button>

              <button
                type="button"
                onClick={() => setIsNewPlaylist(false)}
                className={`p-3 rounded-lg border text-left flex flex-col gap-1 transition-colors cursor-pointer ${
                  !isNewPlaylist
                    ? "bg-emerald-950/20 border-emerald-500/50 text-emerald-300"
                    : "bg-[#121215] border-zinc-800 text-zinc-400 hover:border-zinc-700"
                }`}
              >
                <div className="flex items-center gap-2 font-medium text-xs">
                  <FolderInput className="w-3.5 h-3.5 text-emerald-400" />
                  <span>Append to Existing</span>
                </div>
                <div className="text-[10px] text-zinc-500">
                  Appends tracks to an existing playlist
                </div>
              </button>
            </div>
          </div>

          {/* Conditional Input based on Mode */}
          {isNewPlaylist ? (
            <div className="space-y-1.5">
              <label className="text-[11px] font-medium text-zinc-400">
                New Playlist Name (per D-02)
              </label>
              <input
                type="text"
                value={customTitle}
                onChange={(e) => setCustomTitle(e.target.value)}
                placeholder="Playlist name..."
                className="w-full px-3 py-2 bg-[#121215] border border-zinc-800 rounded-md text-xs text-zinc-200 focus:outline-none focus:border-emerald-500/50"
              />
            </div>
          ) : (
            <div className="space-y-1.5">
              <label className="text-[11px] font-medium text-zinc-400">
                Existing Destination Playlist ID
              </label>
              <input
                type="text"
                value={targetPlaylistId}
                onChange={(e) => setTargetPlaylistId(e.target.value)}
                placeholder="e.g. 37i9dQZF1DXcBWIGoYBM5M or PL..."
                className="w-full px-3 py-2 bg-[#121215] border border-zinc-800 rounded-md text-xs text-zinc-200 focus:outline-none focus:border-emerald-500/50"
              />
            </div>
          )}

          {/* Concurrency Selector per EXEC-02 */}
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <label className="text-xs font-semibold text-zinc-300 flex items-center gap-1.5">
                <Cpu className="w-3.5 h-3.5 text-zinc-400" />
                <span>Worker Concurrency</span>
              </label>
              <span className="text-xs font-mono text-emerald-400 font-semibold">
                {concurrency} {concurrency === 1 ? "worker" : "workers"}
              </span>
            </div>
            <div className="flex items-center gap-2">
              {[1, 2, 4, 6, 8].map((c) => (
                <button
                  key={c}
                  type="button"
                  onClick={() => setConcurrency(c)}
                  className={`flex-1 py-1.5 rounded text-xs font-medium border transition-colors cursor-pointer ${
                    concurrency === c
                      ? "bg-emerald-500 text-black border-emerald-400 font-bold"
                      : "bg-[#121215] border-zinc-800 text-zinc-400 hover:text-zinc-200 hover:border-zinc-700"
                  }`}
                >
                  {c} {c === 4 && <span className="text-[9px] block text-inherit font-normal">(Default)</span>}
                </button>
              ))}
            </div>
          </div>

          {/* Breakdown Stats */}
          <div className="grid grid-cols-3 gap-2 text-center pt-1">
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

          {/* Safety Notice per D-13 & SAFE-01 */}
          <div className="p-3 rounded-lg bg-emerald-950/30 border border-emerald-500/20 text-emerald-300 text-xs flex items-start gap-2.5">
            <ShieldCheck className="w-4 h-4 text-emerald-400 flex-shrink-0 mt-0.5" />
            <div className="text-[11px] leading-relaxed">
              <span className="font-semibold text-emerald-300">1-Click Snapshot Rollback: </span>
              SoundShift will transactionally write an immutable local snapshot to SQLite prior to mutations, allowing instant 1-click rollback at any time.
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
            onClick={handleConfirm}
            disabled={!isNewPlaylist && !targetPlaylistId.trim()}
            className="flex items-center gap-1.5 px-4 py-2 rounded bg-emerald-500 hover:bg-emerald-400 text-black font-semibold text-xs transition-colors cursor-pointer shadow-md disabled:opacity-40 disabled:cursor-not-allowed"
          >
            <span>Confirm & Start Transfer</span>
            <ArrowRight className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>
    </div>
  );
};
