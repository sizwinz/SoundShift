import React, { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  RotateCcw,
  AlertTriangle,
  Trash2,
  ListMinus,
  CheckCircle2,
  Loader2,
} from "lucide-react";
import { TransferHistoryEntry } from "../../types/transfer";
import { isTauri } from "../../utils/tauri";
import { Button, Modal, Badge } from "../ui";

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

  if (!snapshot) return null;

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
    <Modal
      isOpen={isOpen}
      onClose={onClose}
      disableDismiss={isSubmitting}
      title="Rollback Migration"
      description="1-Click snapshot restoration via local mutation logs."
      titleId="rollback-modal-title"
      maxWidth="max-w-md"
      accent="danger"
      icon={
        <div className="w-8 h-8 rounded-lg bg-rose-950/60 border border-rose-500/30 flex items-center justify-center text-rose-400">
          <RotateCcw className="w-4 h-4" />
        </div>
      }
      footer={
        <>
          <Button variant="ghost" onClick={onClose} disabled={isSubmitting}>
            Cancel
          </Button>
          <Button variant="danger" onClick={handleConfirm} disabled={isSubmitting}>
            {isSubmitting && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
            <span>Confirm Rollback</span>
          </Button>
        </>
      }
    >
      <div className="space-y-4 text-xs">
        <div className="p-3.5 rounded-xl bg-[#111116] border border-white/[0.08] space-y-2">
          <div className="flex items-center justify-between gap-2">
            <span className="font-semibold text-zinc-200 truncate">
              {snapshot.source_playlist_name}
            </span>
            <Badge variant="neutral" className="capitalize shrink-0">
              {snapshot.target_service}
            </Badge>
          </div>

          <div className="text-[11px] text-zinc-400 flex items-center justify-between gap-2">
            <span>Migrated: {formattedDate}</span>
            <span className="text-rose-400 font-mono font-medium tabular-nums shrink-0">
              {snapshot.added_tracks_count} tracks to remove
            </span>
          </div>
        </div>

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
                    : "bg-[#111116] border-white/[0.08] text-zinc-400 hover:border-white/[0.16]"
                }`}
              >
                <Trash2 className="w-4 h-4 text-rose-400 shrink-0 mt-0.5" />
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
                    : "bg-[#111116] border-white/[0.08] text-zinc-400 hover:border-white/[0.16]"
                }`}
              >
                <ListMinus className="w-4 h-4 text-rose-400 shrink-0 mt-0.5" />
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
            <CheckCircle2 className="w-4 h-4 text-emerald-400 shrink-0 mt-0.5" />
            <div>
              <span className="font-semibold text-emerald-200">Non-Destructive Safety Guarantee: </span>
              Only the {snapshot.added_tracks_count} tracks added during this migration will be removed from &lsquo;{snapshot.source_playlist_name}&rsquo;. Your {snapshot.pre_existing_count} pre-existing tracks will remain untouched.
            </div>
          </div>
        )}

        <div className="p-3 rounded-lg bg-rose-950/30 border border-rose-500/20 text-rose-300 text-[11px] flex items-start gap-2">
          <AlertTriangle className="w-4 h-4 text-rose-400 shrink-0 mt-0.5" />
          <span>
            This operation cannot be undone. Tracks removed from {snapshot.target_service} will need to be re-transferred.
          </span>
        </div>

        {error && (
          <div role="alert" className="p-2.5 rounded-lg bg-rose-950/60 border border-rose-500/40 text-rose-300 text-xs">
            {error}
          </div>
        )}
      </div>
    </Modal>
  );
};
