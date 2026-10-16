import {
  createContext,
  useContext,
  useState,
  useEffect,
  useCallback,
  useRef,
  ReactNode,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isTauri } from "../utils/tauri";
import {
  TransferStage,
  LifecycleUpdate,
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
  controlPending: boolean;
  startTransfer: (config: BatchTransferConfig) => Promise<void>;
  pauseTransfer: () => Promise<void>;
  resumeTransfer: () => Promise<void>;
  cancelTransfer: () => Promise<void>;
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

  const activeJobId = useRef<string | null>(null);
  const lifecycle = useRef<LifecycleUpdate | null>(null);
  const requestToken = useRef(0);
  const listenersReady = useRef<Promise<boolean> | null>(null);
  const [controlPending, setControlPending] = useState(false);
  const pendingControlJob = useRef<string | null>(null);
  const isTerminal = (state: LifecycleUpdate["state"]) => ["cancelled", "completed_verified", "recovery_required"].includes(state);
  const applyLifecycle = useCallback((update: LifecycleUpdate) => {
    if (update.job_id !== activeJobId.current) return;
    const previous = lifecycle.current;
    if (previous && (update.event_seq <= previous.event_seq || isTerminal(previous.state))) return;
    lifecycle.current = update;
    const stages: Record<LifecycleUpdate["state"], TransferStage> = {
      starting: "starting", running: "transferring", pausing: "pausing", paused: "paused",
      cancelling: "cancelling", cancelled: "cancelled", completed_verified: "completed", recovery_required: "recovery_required",
    };
    setStage(stages[update.state]);
  }, []);

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
            if (payload.job_id !== activeJobId.current) return;
            if (lifecycle.current && isTerminal(lifecycle.current.state)) return;
            setProgress((prev) => prev && prev.processed > payload.processed ? prev : payload);
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

        const uState = await listen<LifecycleUpdate>("job:state_changed", (event) => applyLifecycle(event.payload));
        if (!isMounted) uState(); else cleanups.push(uState);

        const uLog = await listen<TransferLogPayload>("transfer:log", (event) => {
          if (event.payload.job_id !== activeJobId.current) return;
          setLogs((prev) => [event.payload, ...prev.slice(0, 199)]);
        });

        if (!isMounted) {
          uLog();
        } else {
          cleanups.push(uLog);
        }

        const uAudit = await listen<{ job_id: string; audit: AuditResult }>("transfer:audit", (event) => {
          if (event.payload.job_id !== activeJobId.current) return;
          setAuditResult(event.payload.audit);
        });

        if (!isMounted) {
          uAudit();
        } else {
          cleanups.push(uAudit);
        }
        return isMounted;
      } catch (e) {
        setError("Transfer updates are unavailable. Review recorded job state before taking further action.");
        return false;
      }
    };

    listenersReady.current = setupListeners();

    return () => {
      isMounted = false;
      cleanups.forEach((c) => c());
    };
  }, [applyLifecycle]);

  const startTransfer = useCallback(async (config: BatchTransferConfig) => {
    activeJobId.current = config.job_id;
    lifecycle.current = null;
    requestToken.current += 1;
    pendingControlJob.current = null;
    setControlPending(false);
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
      if (!listenersReady.current || !await listenersReady.current) throw new Error("Transfer lifecycle updates are unavailable; no transfer was started.");
      const result = await invoke<BatchTransferSummary>("start_batch_transfer", {
        config,
      });
      if (activeJobId.current !== config.job_id) return;
      setSummary(result);
      applyLifecycle({ job_id: result.job_id, state: result.state, event_seq: result.event_seq });
      setProgress((previous) => ({
        job_id: result.job_id, stage: result.state,
        total: result.total_tracks,
        processed: result.is_cancelled ? result.successful_tracks + result.failed_tracks + (result.skipped_duplicates ?? 0) : result.total_tracks,
        successful: result.successful_tracks, failed: result.failed_tracks,
        current_track: null, worker_id: 0, latency_ms: previous?.latency_ms ?? 0, http_status: 200, is_retry: false,
      }));
      if (result.audit) {
        setAuditResult(result.audit);
      }
    } catch (err) {
      if (activeJobId.current !== config.job_id) return;
      const msg = err instanceof Error ? err.message : String(err);
      // Lifecycle events can mutate the ref while invoke is awaited.
      const latest = lifecycle.current as LifecycleUpdate | null;
      if (latest?.state === "completed_verified" || latest?.state === "cancelled") return;
      setError(msg);
      if (latest) lifecycle.current = { ...latest, state: "recovery_required" };
      setStage(latest ? "recovery_required" : "failed");
    }
  }, [applyLifecycle]);

  const requestControl = useCallback(async (command: string) => {
    const jobId = activeJobId.current;
    if (!jobId || !isTauri() || pendingControlJob.current === jobId) return;
    pendingControlJob.current = jobId;
    const token = ++requestToken.current;
    setControlPending(true);
    setError(null);
    try {
      const update = await invoke<LifecycleUpdate>(command, { jobId });
      applyLifecycle(update);
    } catch (err) {
      if (activeJobId.current === jobId && !(lifecycle.current && isTerminal(lifecycle.current.state))) {
        setError(err instanceof Error ? err.message : String(err));
      }
    } finally {
      if (token === requestToken.current) { pendingControlJob.current = null; setControlPending(false); }
    }
  }, [applyLifecycle]);
  const pauseTransfer = useCallback(() => requestControl("pause_batch_transfer"), [requestControl]);
  const resumeTransfer = useCallback(() => requestControl("resume_batch_transfer"), [requestControl]);
  const cancelTransfer = useCallback(() => requestControl("cancel_batch_transfer"), [requestControl]);

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
    activeJobId.current = null;
    lifecycle.current = null;
    requestToken.current += 1;
    pendingControlJob.current = null;
    setControlPending(false);
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
        controlPending,
        startTransfer,
        pauseTransfer,
        resumeTransfer,
        cancelTransfer,
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
