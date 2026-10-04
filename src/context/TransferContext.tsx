import {
  createContext,
  useContext,
  useState,
  useEffect,
  useCallback,
  ReactNode,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isTauri } from "../utils/tauri";
import {
  TransferStage,
  TransferProgressPayload,
  TransferLogPayload,
  AuditResult,
  BatchTransferConfig,
  BatchTransferSummary,
} from "../types/transfer";

export interface TrackStreamEvent {
  id: string;
  track: string;
  workerId: number;
  latencyMs: number;
  status: number;
  timestamp: number;
}

interface TransferContextType {
  activeConfig: BatchTransferConfig | null;
  stage: TransferStage;
  progress: TransferProgressPayload | null;
  logs: TransferLogPayload[];
  auditResult: AuditResult | null;
  summary: BatchTransferSummary | null;
  trackEvents: TrackStreamEvent[];
  isDrawerOpen: boolean;
  isDrawerExpanded: boolean;
  error: string | null;
  startTransfer: (config: BatchTransferConfig) => Promise<void>;
  pauseTransfer: () => Promise<void>;
  resumeTransfer: () => Promise<void>;
  cancelTransfer: () => Promise<void>;
  retryFailedTracks: () => Promise<void>;
  toggleDrawerExpanded: () => void;
  setDrawerExpanded: (expanded: boolean) => void;
  closeDrawer: () => void;
  openDrawer: () => void;
  resetTransfer: () => void;
}

const TransferContext = createContext<TransferContextType | null>(null);

export function TransferProvider({ children }: { children: ReactNode }) {
  const [activeConfig, setActiveConfig] = useState<BatchTransferConfig | null>(null);
  const [stage, setStage] = useState<TransferStage>("idle");
  const [progress, setProgress] = useState<TransferProgressPayload | null>(null);
  const [logs, setLogs] = useState<TransferLogPayload[]>([]);
  const [auditResult, setAuditResult] = useState<AuditResult | null>(null);
  const [summary, setSummary] = useState<BatchTransferSummary | null>(null);
  const [trackEvents, setTrackEvents] = useState<TrackStreamEvent[]>([]);
  const [isDrawerOpen, setIsDrawerOpen] = useState(false);
  const [isDrawerExpanded, setIsDrawerExpanded] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Subscribe to Tauri events
  useEffect(() => {
    if (!isTauri()) return;

    let isMounted = true;
    const cleanups: Array<() => void> = [];

    const setupListeners = async () => {
      try {
        const uProgress = await listen<TransferProgressPayload>(
          "transfer:progress",
          (event) => {
            const payload = event.payload;
            setProgress(payload);
            if (
              payload.stage === "transferring" ||
              payload.stage === "completed" ||
              payload.stage === "cancelled" ||
              payload.stage === "failed"
            ) {
              setStage(payload.stage as TransferStage);
            }

            if (payload.current_track) {
              setTrackEvents((prev) => [
                {
                  id: `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
                  track: payload.current_track || "Unknown Track",
                  workerId: payload.worker_id,
                  latencyMs: payload.latency_ms,
                  status: payload.http_status,
                  timestamp: Date.now(),
                },
                ...prev.slice(0, 99),
              ]);
            }
          }
        );

        if (!isMounted) {
          uProgress();
        } else {
          cleanups.push(uProgress);
        }

        const uLog = await listen<TransferLogPayload>("transfer:log", (event) => {
          setLogs((prev) => [event.payload, ...prev.slice(0, 199)]);
        });

        if (!isMounted) {
          uLog();
        } else {
          cleanups.push(uLog);
        }

        const uAudit = await listen<AuditResult>("transfer:audit", (event) => {
          setAuditResult(event.payload);
        });

        if (!isMounted) {
          uAudit();
        } else {
          cleanups.push(uAudit);
        }
      } catch (e) {
        console.error("Failed to setup transfer listeners:", e);
      }
    };

    setupListeners();

    return () => {
      isMounted = false;
      cleanups.forEach((c) => c());
    };
  }, []);

  const startTransfer = useCallback(async (config: BatchTransferConfig) => {
    setActiveConfig(config);
    setStage("starting");
    setProgress(null);
    setLogs([]);
    setTrackEvents([]);
    setAuditResult(null);
    setSummary(null);
    setError(null);
    setIsDrawerOpen(true);
    setIsDrawerExpanded(true);

    if (!isTauri()) {
      setError("Native desktop runtime unavailable");
      setStage("failed");
      return;
    }

    try {
      const result = await invoke<BatchTransferSummary>("start_batch_transfer", {
        config,
      });
      setSummary(result);
      if (result.is_cancelled) {
        setStage("cancelled");
      } else {
        setStage("completed");
      }
      if (result.audit) {
        setAuditResult(result.audit);
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg);
      setStage("failed");
    }
  }, []);

  const pauseTransfer = useCallback(async () => {
    if (!activeConfig || !isTauri()) return;
    try {
      await invoke("pause_batch_transfer", { jobId: activeConfig.job_id });
      setStage("paused");
    } catch (err) {
      console.error("Failed to pause transfer:", err);
    }
  }, [activeConfig]);

  const resumeTransfer = useCallback(async () => {
    if (!activeConfig || !isTauri()) return;
    try {
      await invoke("resume_batch_transfer", { jobId: activeConfig.job_id });
      setStage("transferring");
    } catch (err) {
      console.error("Failed to resume transfer:", err);
    }
  }, [activeConfig]);

  const cancelTransfer = useCallback(async () => {
    if (!activeConfig || !isTauri()) return;
    try {
      await invoke("cancel_batch_transfer", { jobId: activeConfig.job_id });
      setStage("cancelled");
    } catch (err) {
      console.error("Failed to cancel transfer:", err);
    }
  }, [activeConfig]);

  const retryFailedTracks = useCallback(async () => {
    if (!activeConfig || !summary || summary.failed_tracks === 0) return;
    // Identify tracks that were not successfully transferred
    // For non-blocking retry per D-05, slice remaining or failed tracks
    const failedTrackNames = new Set(
      trackEvents
        .filter((event) => event.status >= 400)
        .map((event) => event.track)
    );
    const failedTracks = activeConfig.tracks.filter((track) =>
      failedTrackNames.has(track.title)
    );
    if (failedTracks.length === 0) return;

    const retryConfig: BatchTransferConfig = {
      ...activeConfig,
      job_id: `retry_${Date.now()}_${activeConfig.job_id.slice(-6)}`,
      tracks: failedTracks,
      is_new_playlist: false,
      target_playlist_id: summary.target_playlist_id,
    };

    await startTransfer(retryConfig);
  }, [activeConfig, summary, trackEvents, startTransfer]);

  const toggleDrawerExpanded = useCallback(() => {
    setIsDrawerExpanded((prev) => !prev);
  }, []);

  const setDrawerExpanded = useCallback((expanded: boolean) => {
    setIsDrawerExpanded(expanded);
  }, []);

  const closeDrawer = useCallback(() => {
    setIsDrawerOpen(false);
  }, []);

  const openDrawer = useCallback(() => {
    setIsDrawerOpen(true);
  }, []);

  const resetTransfer = useCallback(() => {
    setStage("idle");
    setProgress(null);
    setLogs([]);
    setTrackEvents([]);
    setAuditResult(null);
    setSummary(null);
    setError(null);
    setIsDrawerOpen(false);
    setIsDrawerExpanded(false);
    setActiveConfig(null);
  }, []);

  return (
    <TransferContext.Provider
      value={{
        activeConfig,
        stage,
        progress,
        logs,
        auditResult,
        summary,
        trackEvents,
        isDrawerOpen,
        isDrawerExpanded,
        error,
        startTransfer,
        pauseTransfer,
        resumeTransfer,
        cancelTransfer,
        retryFailedTracks,
        toggleDrawerExpanded,
        setDrawerExpanded,
        closeDrawer,
        openDrawer,
        resetTransfer,
      }}
    >
      {children}
    </TransferContext.Provider>
  );
}

export function useTransfer() {
  const context = useContext(TransferContext);
  if (!context) {
    throw new Error("useTransfer must be used within a TransferProvider");
  }
  return context;
}
