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
import { Card, Button, Badge } from "../ui";

interface TransferHistoryProps {
  onBrowsePlaylists?: () => void;
}

export const TransferHistory: React.FC<TransferHistoryProps> = ({
  onBrowsePlaylists,
}) => {
  const [history, setHistory] = useState<TransferHistoryEntry[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [recoveryNotice, setRecoveryNotice] = useState<string | null>(null);
  const [isAcknowledging, setIsAcknowledging] = useState(false);
  const [hasReviewedProviders, setHasReviewedProviders] = useState(false);
  const [activeRollbackSnapshot, setActiveRollbackSnapshot] =
    useState<TransferHistoryEntry | null>(null);
  const [auditingJobId, setAuditingJobId] = useState<string | null>(null);

  const loadHistory = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    if (!isTauri()) {
      setIsLoading(false);
      return;
    }

    try {
      const [entries, notice] = await Promise.all([
        invoke<TransferHistoryEntry[]>("get_transfer_history"),
        invoke<string | null>("get_storage_recovery_notice"),
      ]);
      setHistory(entries);
      setRecoveryNotice(notice);
    } catch (err) {
      setError(String(err));
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    loadHistory();
  }, [loadHistory]);

  const acknowledgeRecovery = async () => {
    if (!hasReviewedProviders) return;
    setIsAcknowledging(true);
    try {
      await invoke("acknowledge_storage_recovery", { reviewed: hasReviewedProviders });
      setHasReviewedProviders(false);
      await loadHistory();
    } catch (err) {
      setError(String(err));
    } finally {
      setIsAcknowledging(false);
    }
  };

  const handleAudit = async (entry: TransferHistoryEntry) => {
    setAuditingJobId(entry.job_id);
    setError(null);
    try {
      const result = await invoke<AuditResult>("audit_playlist_integrity", {
        targetService: entry.target_service,
        targetPlaylistId: entry.target_playlist_id,
        expectedTrackIds: [],
        snapshotId: entry.snapshot_id,
      });

      setHistory((prev) =>
        prev.map((item) =>
          item.job_id === entry.job_id ? { ...item, audit_status: result } : item
        )
      );
    } catch (err) {
      setError(String(err));
    } finally {
      setAuditingJobId(null);
    }
  };

  const totalMigrations = history.length;
  const totalTracks = history.reduce((acc, h) => acc + h.matched_tracks, 0);
  const totalRolledBack = history.filter((h) => h.is_rolled_back).length;

  return (
    <div className="flex flex-col gap-6 max-w-[1440px] mx-auto w-full pb-16">
      {/* Header */}
      <div className="flex items-center justify-between pb-3 border-b border-white/[0.08]">
        <div>
          <h1 className="text-base font-semibold text-zinc-100 flex items-center gap-2">
            <History className="w-4 h-4 text-emerald-400" />
            <span>Transfer History & Snapshots</span>
          </h1>
          <p className="text-xs text-zinc-400 mt-0.5">
            Immutable pre-mutation audit records, 1-click rollback, and post-transfer integrity status.
          </p>
        </div>

        <Button
          variant="secondary"
          size="sm"
          onClick={loadHistory}
          disabled={isLoading}
        >
          <RefreshCw className={`w-3.5 h-3.5 ${isLoading ? "animate-spin" : ""}`} />
          <span>Refresh</span>
        </Button>
      </div>

      {error && (
        <div role="alert" className="rounded-xl border border-rose-500/40 bg-rose-950/20 p-3.5 text-xs text-rose-300">
          {error}
        </div>
      )}

      {recoveryNotice && (
        <div role="alert" className="rounded-xl border border-amber-500/40 bg-amber-950/20 p-4 text-xs text-amber-200">
          <p className="leading-relaxed">{recoveryNotice}</p>
          <label className="mt-3 flex items-start gap-2.5 text-xs text-zinc-300 cursor-pointer">
            <input
              type="checkbox"
              checked={hasReviewedProviders}
              onChange={(event) => setHasReviewedProviders(event.target.checked)}
              disabled={isAcknowledging}
              className="mt-0.5 rounded border-white/[0.2] bg-zinc-900 accent-amber-500"
            />
            <span>I reviewed my provider playlists, including changes after this backup. I understand that previous jobs still require recovery review.</span>
          </label>
          <Button
            variant="amber"
            size="xs"
            onClick={acknowledgeRecovery}
            disabled={isAcknowledging || !hasReviewedProviders}
            className="mt-3"
          >
            {isAcknowledging ? "Saving review..." : "I reviewed my provider playlists"}
          </Button>
        </div>
      )}

      {/* Metrics Cards */}
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
        <Card className="p-4 bg-[#0a0a0e]">
          <div className="text-[10px] uppercase font-mono tracking-wider text-zinc-500 font-semibold">Total Migrations</div>
          <div className="text-xl font-bold text-zinc-100 font-mono mt-1 tabular-nums">
            {totalMigrations}
          </div>
        </Card>

        <Card className="p-4 bg-[#0a0a0e]">
          <div className="text-[10px] uppercase font-mono tracking-wider text-zinc-500 font-semibold">Songs Transferred</div>
          <div className="text-xl font-bold text-emerald-400 font-mono mt-1 tabular-nums">
            {totalTracks}
          </div>
        </Card>

        <Card className="p-4 bg-[#0a0a0e]">
          <div className="text-[10px] uppercase font-mono tracking-wider text-zinc-500 font-semibold">Rolled Back</div>
          <div className="text-xl font-bold text-amber-400 font-mono mt-1 tabular-nums">
            {totalRolledBack}
          </div>
        </Card>

        <Card className="p-4 bg-[#0a0a0e]">
          <div className="text-[10px] uppercase font-mono tracking-wider text-zinc-500 font-semibold">Local Integrity</div>
          <div className="text-xl font-bold text-emerald-400 font-mono mt-1 flex items-center gap-1.5">
            <ShieldCheck className="w-5 h-5 text-emerald-400" />
            <span className="font-sans text-sm font-semibold">Active</span>
          </div>
        </Card>
      </div>

      {/* History Items List */}
      {history.length === 0 ? (
        <Card className="p-12 text-center text-zinc-400 bg-[#09090d]/40 border-dashed border-white/[0.1]">
          <History className="w-10 h-10 text-zinc-600 mx-auto mb-3" />
          <div className="text-sm font-semibold text-zinc-200 mb-1">
            No Migration History Found
          </div>
          <p className="text-xs text-zinc-500 max-w-sm mx-auto mb-5 leading-relaxed">
            Completed transfers will generate local SQLite snapshots allowing instant 1-click rollback and integrity audit.
          </p>
          {onBrowsePlaylists && (
            <Button
              variant="primary"
              size="sm"
              onClick={onBrowsePlaylists}
            >
              <span>Browse Playlists</span>
              <ArrowRight className="w-3.5 h-3.5" />
            </Button>
          )}
        </Card>
      ) : (
        <div className="space-y-3">
          {history.map((entry) => {
            const formattedDate = new Date(entry.created_at * 1000).toLocaleString();
            const isAuditing = auditingJobId === entry.job_id;

            return (
              <Card
                key={entry.job_id}
                className={`p-4 transition-all ${
                  entry.is_rolled_back
                    ? "bg-[#08080c]/50 border-white/[0.04] opacity-75"
                    : "bg-[#0b0b10] border-white/[0.08] hover:border-white/[0.16]"
                }`}
              >
                <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
                  <div className="flex items-start gap-3 min-w-0">
                    <div className="w-10 h-10 rounded-xl bg-white/[0.04] border border-white/[0.08] flex items-center justify-center text-zinc-300 shrink-0 mt-0.5">
                      <Music className="w-4 h-4 text-emerald-400" />
                    </div>

                    <div className="min-w-0">
                      <div className="flex items-center gap-2 flex-wrap">
                        <span className="text-sm font-semibold text-zinc-100 truncate">
                          {entry.source_playlist_name}
                        </span>

                        {entry.recovery_required ? (
                          <Badge variant="amber" icon={<AlertTriangle className="w-3 h-3" />}>
                            Recovery required
                          </Badge>
                        ) : entry.is_rolled_back ? (
                          <Badge variant="duplicate" icon={<RotateCcw className="w-3 h-3 text-amber-400" />}>
                            Rolled Back
                          </Badge>
                        ) : entry.audit_status?.is_verified ? (
                          <Badge variant="exact" icon={<ShieldCheck className="w-3 h-3 text-emerald-400" />}>
                            Audit Verified
                          </Badge>
                        ) : entry.audit_status ? (
                          <Badge variant="unmatched" icon={<AlertTriangle className="w-3 h-3 text-rose-400" />}>
                            Discrepancy
                          </Badge>
                        ) : (
                          <Badge variant="connected" icon={<CheckCircle2 className="w-3 h-3 text-emerald-400" />}>
                            {entry.job_state === "completed_verified" ? "Completed and verified" : entry.job_state.replaceAll("_", " ")}
                          </Badge>
                        )}
                      </div>

                      <div className="flex items-center gap-2 text-xs text-zinc-400 mt-1 font-mono">
                        <span className="capitalize">{entry.source_service}</span>
                        <ArrowRight className="w-3 h-3 text-zinc-600" />
                        <span className="capitalize">{entry.target_service}</span>
                        <span className="text-zinc-600">•</span>
                        <span className="tabular-nums font-semibold text-zinc-200">{entry.matched_tracks} tracks</span>
                        <span className="text-zinc-600">•</span>
                        <span className="text-zinc-500 font-sans">{formattedDate}</span>
                      </div>
                    </div>
                  </div>

                  <div className="flex items-center gap-2 shrink-0 self-end sm:self-center">
                    {!entry.is_rolled_back && !entry.recovery_required && (
                      <Button
                        variant="secondary"
                        size="xs"
                        onClick={() => handleAudit(entry)}
                        disabled={isAuditing}
                      >
                        <RefreshCw className={`w-3 h-3 ${isAuditing ? "animate-spin" : ""}`} />
                        <span>Audit</span>
                      </Button>
                    )}

                    {!entry.is_rolled_back && !entry.recovery_required && (
                      <Button
                        variant="danger"
                        size="xs"
                        onClick={() => setActiveRollbackSnapshot(entry)}
                      >
                        <RotateCcw className="w-3 h-3" />
                        <span>Rollback</span>
                      </Button>
                    )}
                  </div>
                </div>
              </Card>
            );
          })}
        </div>
      )}

      {/* Rollback Modal */}
      {activeRollbackSnapshot && (
        <RollbackModal
          isOpen={Boolean(activeRollbackSnapshot)}
          snapshot={activeRollbackSnapshot}
          onClose={() => setActiveRollbackSnapshot(null)}
          onRollbackSuccess={() => {
            setActiveRollbackSnapshot(null);
            loadHistory();
          }}
        />
      )}
    </div>
  );
};
