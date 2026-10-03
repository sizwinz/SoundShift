import React, { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  RotateCcw,
  AlertTriangle,
  X,
  Trash2,
  ListMinus,
  CheckCircle2,
  Loader2,
} from "lucide-react";
import { TransferHistoryEntry } from "../../types/transfer";
import { isTauri } from "../../utils/tauri";

interface RollbackModalProps {
  isOpen: boolean;
  onClose: () => void;
  snapshot: TransferHistoryEntry | null;
  onRollbackSuccess: () => void;
}

export const RollbackModal: React.FC<RollbackModalProps> = ({
  isOpen,
  onClose,
  snapshot,
  onRollbackSuccess,
}) => {
  const [deleteEntirePlaylist, setDeleteEntirePlaylist] = useState(true);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!isOpen || !snapshot) return null;

  const handleConfirm = async () => {
    setIsSubmitting(true);
    setError(null);

    if (!isTauri()) {
      setError("Desktop runtime unavailable");
      setIsSubmitting(false);
      return;
    }

    try {
      await invoke("execute_snapshot_rollback", {
        snapshotId: snapshot.snapshot_id,
        deleteEntirePlaylist: snapshot.is_new_playlist ? deleteEntirePlaylist : false,
      });

      onRollbackSuccess();
      onClose();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg);
    } finally {
      setIsSubmitting(false);
    }
  };

  const formattedDate = new Date(snapshot.created_at * 1000).toLocaleString();

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/80 backdrop-blur-xs animate-in fade-in duration-150"
      role="dialog"
      aria-modal="true"
      aria-labelledby="rollback-modal-title"
    >
      <div className="w-full max-w-md bg-[#09090b] border border-rose-500/30 rounded-xl shadow-2xl overflow-hidden animate-in zoom-in-95 duration-150">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-[#27272a] bg-[#0c0c0e]">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-lg bg-rose-950/60 border border-rose-500/30 flex items-center justify-center text-rose-400">
              <RotateCcw className="w-4 h-4" />
            </div>
            <div>
              <h2 id="rollback-modal-title" className="text-sm font-bold text-zinc-100">
                Rollback Migration
              </h2>
              <p className="text-[11px] text-zinc-500">
                1-Click snapshot restoration via local mutation logs.
              </p>
            </div>
          </div>

          <button
            type="button"
            onClick={onClose}
            disabled={isSubmitting}
            className="p-1 rounded-md text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors cursor-pointer disabled:opacity-40"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Content */}
        <div className="p-5 space-y-4 text-xs">
          {/* Snapshot Summary Box */}
          <div className="p-3.5 rounded-lg bg-[#121215] border border-zinc-800 space-y-2">
            <div className="flex items-center justify-between">
              <span className="font-semibold text-zinc-200">
                {snapshot.source_playlist_name}
              </span>
              <span className="capitalize px-2 py-0.5 rounded bg-zinc-800 text-[10px] text-zinc-300 font-medium">
                {snapshot.target_service}
              </span>
            </div>

            <div className="text-[11px] text-zinc-400 flex items-center justify-between">
              <span>Migrated: {formattedDate}</span>
              <span className="text-rose-400 font-mono font-medium">
                {snapshot.added_tracks_count} tracks to remove
              </span>
            </div>
          </div>

          {/* Rollback Mode Choice per D-07 */}
          {snapshot.is_new_playlist ? (
            <div className="space-y-2">
              <label className="font-semibold text-zinc-300 block">
                Rollback Options
              </label>

              <div className="space-y-2">
                <button
                  type="button"
                  onClick={() => setDeleteEntirePlaylist(true)}
                  className={`w-full p-3 rounded-lg border text-left flex items-start gap-3 transition-colors cursor-pointer ${
                    deleteEntirePlaylist
                      ? "bg-rose-950/20 border-rose-500/50 text-rose-300"
                      : "bg-[#121215] border-zinc-800 text-zinc-400 hover:border-zinc-700"
                  }`}
                >
                  <Trash2 className="w-4 h-4 text-rose-400 flex-shrink-0 mt-0.5" />
                  <div>
                    <div className="font-semibold text-xs">
                      Delete Entire Playlist (Recommended)
                    </div>
                    <div className="text-[10px] text-zinc-500 mt-0.5">
                      Deletes the newly created destination playlist container and all songs inside it.
                    </div>
                  </div>
                </button>

                <button
                  type="button"
                  onClick={() => setDeleteEntirePlaylist(false)}
                  className={`w-full p-3 rounded-lg border text-left flex items-start gap-3 transition-colors cursor-pointer ${
                    !deleteEntirePlaylist
                      ? "bg-rose-950/20 border-rose-500/50 text-rose-300"
                      : "bg-[#121215] border-zinc-800 text-zinc-400 hover:border-zinc-700"
                  }`}
                >
                  <ListMinus className="w-4 h-4 text-rose-400 flex-shrink-0 mt-0.5" />
                  <div>
                    <div className="font-semibold text-xs">
                      Remove Added Tracks Only
                    </div>
                    <div className="text-[10px] text-zinc-500 mt-0.5">
                      Removes only the transferred songs, leaving the empty playlist container intact.
                    </div>
                  </div>
                </button>
              </div>
            </div>
          ) : (
            <div className="p-3 rounded-lg bg-emerald-950/20 border border-emerald-500/30 text-emerald-300 text-[11px] leading-relaxed flex items-start gap-2.5">
              <CheckCircle2 className="w-4 h-4 text-emerald-400 flex-shrink-0 mt-0.5" />
              <div>
                <span className="font-semibold text-emerald-200">Non-Destructive Safety Guarantee: </span>
                Only the {snapshot.added_tracks_count} tracks added during this migration will be removed from '{snapshot.source_playlist_name}'. Your {snapshot.pre_existing_count} pre-existing tracks will remain untouched.
              </div>
            </div>
          )}

          {/* Warning Banner */}
          <div className="p-3 rounded-lg bg-rose-950/30 border border-rose-500/20 text-rose-300 text-[11px] flex items-start gap-2">
            <AlertTriangle className="w-4 h-4 text-rose-400 flex-shrink-0 mt-0.5" />
            <span>
              This operation cannot be undone. Tracks removed from {snapshot.target_service} will need to be re-transferred.
            </span>
          </div>

          {error && (
            <div className="p-2.5 rounded bg-rose-950/60 border border-rose-500/40 text-rose-300 text-xs">
              {error}
            </div>
          )}
        </div>

        {/* Footer Actions */}
        <div className="flex items-center justify-end gap-2.5 px-5 py-3.5 border-t border-[#27272a] bg-[#0c0c0e]">
          <button
            type="button"
            onClick={onClose}
            disabled={isSubmitting}
            className="px-3.5 py-1.5 rounded text-xs font-medium text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800 transition-colors cursor-pointer disabled:opacity-40"
          >
            Cancel
          </button>

          <button
            type="button"
            onClick={handleConfirm}
            disabled={isSubmitting}
            className="flex items-center gap-1.5 px-4 py-2 rounded bg-rose-600 hover:bg-rose-500 text-white font-semibold text-xs transition-colors cursor-pointer shadow-md disabled:opacity-50"
          >
            {isSubmitting && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
            <span>Confirm Rollback</span>
          </button>
        </div>
      </div>
    </div>
  );
};
