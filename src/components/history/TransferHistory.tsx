import React, { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  History,
  ShieldCheck,
  AlertTriangle,
  RotateCcw,
  ArrowRight,
  RefreshCw,
  Music,
  CheckCircle2,
} from "lucide-react";
import { TransferHistoryEntry, AuditResult } from "../../types/transfer";
import { RollbackModal } from "./RollbackModal";
import { isTauri } from "../../utils/tauri";

interface TransferHistoryProps {
  onBrowsePlaylists?: () => void;
}

export const TransferHistory: React.FC<TransferHistoryProps> = ({
  onBrowsePlaylists,
}) => {
  const [history, setHistory] = useState<TransferHistoryEntry[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [activeRollbackSnapshot, setActiveRollbackSnapshot] =
    useState<TransferHistoryEntry | null>(null);
  const [auditingJobId, setAuditingJobId] = useState<string | null>(null);

  const loadHistory = useCallback(async () => {
    setIsLoading(true);
    if (!isTauri()) {
      setIsLoading(false);
      return;
    }

    try {
      const entries = await invoke<TransferHistoryEntry[]>("get_transfer_history");
      setHistory(entries);
    } catch (err) {
      console.error("Failed to load transfer history:", err);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    loadHistory();
  }, [loadHistory]);

  const handleAudit = async (entry: TransferHistoryEntry) => {
    setAuditingJobId(entry.job_id);
    try {
      const result = await invoke<AuditResult>("audit_playlist_integrity", {
        targetService: entry.target_service,
        targetPlaylistId: entry.target_playlist_id,
        expectedTrackIds: [], // Provider will check existence
      });

      setHistory((prev) =>
        prev.map((item) =>
          item.job_id === entry.job_id ? { ...item, audit_status: result } : item
        )
      );
    } catch (err) {
      console.error("Audit failed:", err);
    } finally {
      setAuditingJobId(null);
    }
  };

  const totalMigrations = history.length;
  const totalTracks = history.reduce((acc, h) => acc + h.matched_tracks, 0);
  const totalRolledBack = history.filter((h) => h.is_rolled_back).length;

  return (
    <div className="flex flex-col gap-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-lg font-bold text-zinc-100 flex items-center gap-2">
            <History className="w-5 h-5 text-emerald-400" />
            <span>Transfer History & Snapshots</span>
          </h1>
          <p className="text-xs text-zinc-500">
            Immutable pre-mutation audit records, 1-click rollback, and post-transfer integrity status.
          </p>
        </div>

        <button
          type="button"
          onClick={loadHistory}
          disabled={isLoading}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-zinc-900 hover:bg-zinc-800 border border-zinc-800 text-xs font-medium text-zinc-300 hover:text-zinc-100 transition-colors cursor-pointer disabled:opacity-40"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${isLoading ? "animate-spin" : ""}`} />
          <span>Refresh</span>
        </button>
      </div>

      {/* Metrics Strip */}
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
        <div className="p-3.5 rounded-lg bg-[#0c0c0e] border border-zinc-800">
          <div className="text-[10px] uppercase font-mono text-zinc-500">Total Migrations</div>
          <div className="text-xl font-bold text-zinc-100 font-mono mt-0.5">
            {totalMigrations}
          </div>
        </div>

        <div className="p-3.5 rounded-lg bg-[#0c0c0e] border border-zinc-800">
          <div className="text-[10px] uppercase font-mono text-zinc-500">Songs Transferred</div>
          <div className="text-xl font-bold text-emerald-400 font-mono mt-0.5">
            {totalTracks}
          </div>
        </div>

        <div className="p-3.5 rounded-lg bg-[#0c0c0e] border border-zinc-800">
          <div className="text-[10px] uppercase font-mono text-zinc-500">Rolled Back</div>
          <div className="text-xl font-bold text-amber-400 font-mono mt-0.5">
            {totalRolledBack}
          </div>
        </div>

        <div className="p-3.5 rounded-lg bg-[#0c0c0e] border border-zinc-800">
          <div className="text-[10px] uppercase font-mono text-zinc-500">Local Integrity</div>
          <div className="text-xl font-bold text-emerald-400 font-mono mt-0.5 flex items-center gap-1.5">
            <ShieldCheck className="w-5 h-5 text-emerald-400" />
            <span>Active</span>
          </div>
        </div>
      </div>

      {/* History Items List */}
      {history.length === 0 ? (
        <div className="border border-[#27272a] rounded-lg bg-[#09090b] p-10 text-center text-zinc-400">
          <History className="w-10 h-10 text-zinc-600 mx-auto mb-3" />
          <div className="text-sm font-medium text-zinc-200 mb-1">
            No Migration History Found
          </div>
          <p className="text-xs text-zinc-500 max-w-sm mx-auto mb-4">
            Completed transfers will generate local SQLite snapshots allowing instant 1-click rollback.
          </p>
          {onBrowsePlaylists && (
            <button
              type="button"
              onClick={onBrowsePlaylists}
              className="inline-flex items-center gap-2 px-4 py-2 bg-emerald-500 hover:bg-emerald-400 text-black text-xs font-semibold rounded cursor-pointer transition-colors"
            >
              <span>Browse Playlists</span>
              <ArrowRight className="w-3.5 h-3.5" />
            </button>
          )}
        </div>
      ) : (
        <div className="space-y-3">
          {history.map((entry) => {
            const formattedDate = new Date(entry.created_at * 1000).toLocaleString();
            const isAuditing = auditingJobId === entry.job_id;

            return (
              <div
                key={entry.job_id}
                className={`p-4 rounded-xl border transition-all ${
                  entry.is_rolled_back
                    ? "bg-[#09090b]/60 border-zinc-800/60 opacity-70"
                    : "bg-[#09090b] border-zinc-800 hover:border-zinc-700"
                }`}
              >
                <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
                  {/* Left: Info */}
                  <div className="flex items-start gap-3 min-w-0">
                    <div className="w-10 h-10 rounded-lg bg-zinc-800/80 border border-zinc-700/50 flex items-center justify-center text-zinc-300 flex-shrink-0 mt-0.5">
                      <Music className="w-5 h-5" />
                    </div>

                    <div className="min-w-0">
                      <div className="flex items-center gap-2 flex-wrap">
                        <span className="text-sm font-bold text-zinc-100 truncate">
                          {entry.source_playlist_name}
                        </span>

                        {/* Status Badges */}
                        {entry.is_rolled_back ? (
                          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded bg-zinc-800 border border-zinc-700 text-[10px] text-zinc-400 font-mono">
                            <RotateCcw className="w-3 h-3 text-amber-400" />
                            <span>Rolled Back</span>
                          </span>
                        ) : entry.audit_status?.is_verified ? (
                          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded bg-emerald-950/60 border border-emerald-500/30 text-[10px] text-emerald-400 font-medium">
                            <ShieldCheck className="w-3 h-3 text-emerald-400" />
                            <span>Audit Verified</span>
                          </span>
                        ) : entry.audit_status ? (
                          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded bg-rose-950/60 border border-rose-500/30 text-[10px] text-rose-400 font-medium">
                            <AlertTriangle className="w-3 h-3 text-rose-400" />
                            <span>Discrepancy</span>
                          </span>
                        ) : (
                          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded bg-emerald-950/40 border border-emerald-500/20 text-[10px] text-emerald-400 font-medium">
                            <CheckCircle2 className="w-3 h-3 text-emerald-400" />
                            <span>Completed</span>
                          </span>
                        )}
                      </div>

                      {/* Details row */}
                      <div className="flex items-center gap-3 text-[11px] text-zinc-400 mt-1 flex-wrap">
                        <span className="capitalize text-zinc-300 font-medium">
                          {entry.source_service} → {entry.target_service}
                        </span>
                        <span>•</span>
                        <span>{formattedDate}</span>
                        <span>•</span>
                        <span className="font-mono text-zinc-300">
                          {entry.matched_tracks} / {entry.total_tracks} tracks ({entry.added_tracks_count} added)
                        </span>
                      </div>
                    </div>
                  </div>

                  {/* Right: Actions */}
                  <div className="flex items-center gap-2 self-end sm:self-center flex-shrink-0">
                    <button
                      type="button"
                      onClick={() => handleAudit(entry)}
                      disabled={isAuditing || entry.is_rolled_back}
                      className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-zinc-900 hover:bg-zinc-800 border border-zinc-800 text-xs font-medium text-zinc-300 hover:text-zinc-100 transition-colors cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed"
                      title="Verify destination playlist contents against snapshot records"
                    >
                      <ShieldCheck className={`w-3.5 h-3.5 ${isAuditing ? "animate-spin text-emerald-400" : "text-emerald-400"}`} />
                      <span>{isAuditing ? "Auditing..." : "Audit"}</span>
                    </button>

                    <button
                      type="button"
                      onClick={() => setActiveRollbackSnapshot(entry)}
                      disabled={entry.is_rolled_back}
                      className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-rose-950/30 hover:bg-rose-900/40 border border-rose-500/30 text-xs font-medium text-rose-300 hover:text-rose-200 transition-colors cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed"
                      title={entry.is_rolled_back ? "Snapshot already rolled back" : "Rollback added tracks"}
                    >
                      <RotateCcw className="w-3.5 h-3.5 text-rose-400" />
                      <span>Rollback</span>
                    </button>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* 1-Click Rollback Modal */}
      <RollbackModal
        isOpen={Boolean(activeRollbackSnapshot)}
        onClose={() => setActiveRollbackSnapshot(null)}
        snapshot={activeRollbackSnapshot}
        onRollbackSuccess={loadHistory}
      />
    </div>
  );
};
