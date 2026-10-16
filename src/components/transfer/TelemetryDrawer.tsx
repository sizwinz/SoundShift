import React, { useState } from "react";
import {
  ChevronUp,
  ChevronDown,
  Pause,
  Play,
  XSquare,
  ShieldCheck,
  AlertTriangle,
  Terminal,
  Activity,
} from "lucide-react";
import { useTransfer } from "../../context/TransferContext";

interface TelemetryDrawerProps {
  onNavigateToHistory?: () => void;
}

export const TelemetryDrawer: React.FC<TelemetryDrawerProps> = ({
  onNavigateToHistory,
}) => {
  const {
    activeConfig,
    stage,
    progress,
    logs,
    auditResult,
    summary,
    error,
    controlPending,
    trackEvents,
    isDrawerOpen,
    isDrawerExpanded,
    toggleDrawerExpanded,
    pauseTransfer,
    resumeTransfer,
    cancelTransfer,
    resetTransfer,
  } = useTransfer();

  const [activeTab, setActiveTab] = useState<"stream" | "logs">("stream");
  const [showCancelPrompt, setShowCancelPrompt] = useState(false);

  if (!isDrawerOpen || stage === "idle") {
    return null;
  }

  const processed = progress?.processed ?? 0;
  const total = progress?.total ?? activeConfig?.tracks.length ?? 0;
  const percent = total > 0 ? Math.min(100, Math.round((processed / total) * 100)) : 0;
  const isPaused = stage === "paused";
  const isCompleted = stage === "completed";
  const isCancelled = stage === "cancelled";
  const isFailed = stage === "failed" || stage === "recovery_required";
  const isPending = stage === "pausing" || stage === "cancelling";
  const isFinished = isCompleted || isCancelled || isFailed;

  const handleCancelClick = () => {
    if (isFinished) {
      resetTransfer();
    } else {
      setShowCancelPrompt(true);
    }
  };

  const confirmCancel = async () => {
    setShowCancelPrompt(false);
    await cancelTransfer();
  };

  return (
    <aside
      className={`shrink-0 w-full z-30 bg-[#08080a] border-t border-white/[0.08] shadow-[0_-8px_32px_rgba(0,0,0,0.6)] transition-all duration-300 flex flex-col select-none ${
        isDrawerExpanded ? "h-64 sm:h-72" : "h-14"
      }`}
      aria-label="Migration Telemetry Drawer"
    >
      {/* Top Thin Progress Line */}
      <div className="w-full h-1 bg-white/[0.04] relative overflow-hidden">
        <div
          className={`h-full transition-all duration-300 ${
            isCancelled
              ? "bg-amber-500"
              : isFailed
              ? "bg-rose-500"
              : isCompleted
              ? "bg-emerald-400"
              : isPaused
              ? "bg-amber-400"
              : "bg-emerald-500"
          }`}
          style={{ width: `${percent}%` }}
        />
      </div>

      {/* Docked Activity Bar (Always Visible) */}
      <div className="h-13 px-4 flex items-center justify-between border-b border-white/[0.04] bg-[#0a0a0e]/95">
        {/* Left Status & Current Track */}
        <div className="flex items-center gap-3 min-w-0">
          <div className="flex items-center gap-2">
            <span
              className={`w-2.5 h-2.5 rounded-full ${
                isCompleted
                  ? "bg-emerald-500"
                  : isCancelled
                  ? "bg-amber-500"
                  : isFailed
                  ? "bg-rose-500"
                  : isPaused
                  ? "bg-amber-400 animate-pulse"
                  : "bg-emerald-400 animate-pulse"
              }`}
            />
            <span className="text-[11px] font-bold text-zinc-100 uppercase tracking-wider font-mono">
              {stage.replaceAll("_", " ")}
            </span>
          </div>

          <div className="h-4 w-px bg-white/[0.08] hidden sm:block" />

          {/* Current track or summary */}
          <div className="text-xs text-zinc-400 truncate max-w-xs sm:max-w-md">
            {isPending ? (
              <span className="text-amber-300">Waiting for in-flight requests and checkpoints...</span>
            ) : progress?.current_track ? (
              <span className="text-zinc-200 font-medium">{progress.current_track}</span>
            ) : isCompleted ? (
              <span className="text-emerald-400 font-medium">Migration successfully completed</span>
            ) : isCancelled ? (
              <span className="text-amber-400">Migration stopped by user</span>
            ) : isFailed ? (
              <span className="text-rose-300 truncate">{error || "Transfer failed. Expand for details."}</span>
            ) : (
              <span>Preparing transfer manifest...</span>
            )}
          </div>
        </div>

        {/* Right Stats & Controls */}
        <div className="flex items-center gap-3 shrink-0">
          <div className="text-xs font-mono font-medium text-zinc-300 tabular-nums">
            <span>{processed}</span>
            <span className="text-zinc-600 mx-1">/</span>
            <span>{total}</span>
            <span className="text-emerald-400 ml-1.5 font-semibold">({percent}%)</span>
          </div>

          {/* Worker Controls */}
          {!isFinished ? (
            <div className="flex items-center gap-1.5">
              {isPaused ? (
                <button
                  type="button"
                  onClick={resumeTransfer}
                  disabled={controlPending || isPending}
                  className="flex items-center gap-1 px-2.5 py-1 rounded-md bg-emerald-500/20 hover:bg-emerald-500/30 text-emerald-400 border border-emerald-500/30 text-xs font-medium transition-colors cursor-pointer"
                  title="Resume Transfer"
                >
                  <Play className="w-3 h-3 fill-current" />
                  <span className="hidden sm:inline">Resume</span>
                </button>
              ) : (
                <button
                  type="button"
                  onClick={pauseTransfer}
                  disabled={controlPending || isPending || stage === "starting"}
                  className="flex items-center gap-1 px-2.5 py-1 rounded-md bg-white/[0.06] hover:bg-white/[0.12] text-zinc-300 border border-white/[0.08] text-xs font-medium transition-colors cursor-pointer disabled:opacity-40"
                  title="Pause Transfer"
                >
                  <Pause className="w-3 h-3" />
                  <span className="hidden sm:inline">Pause</span>
                </button>
              )}

              <button
                type="button"
                onClick={handleCancelClick}
                disabled={controlPending || stage === "cancelling" || stage === "starting"}
                className="flex items-center gap-1 px-2.5 py-1 rounded-md bg-zinc-900 hover:bg-rose-950/40 text-zinc-400 hover:text-rose-400 border border-white/[0.08] text-xs font-medium transition-colors cursor-pointer disabled:opacity-40"
                title="Cancel Transfer"
              >
                <XSquare className="w-3 h-3" />
                <span className="hidden sm:inline">Cancel</span>
              </button>
            </div>
          ) : (
            <button
              type="button"
              onClick={resetTransfer}
              className="flex items-center gap-1 px-2.5 py-1 rounded-md bg-white/[0.06] hover:bg-white/[0.1] text-zinc-300 border border-white/[0.08] text-xs font-medium transition-colors cursor-pointer"
              title="Dismiss Activity"
            >
              <span>Dismiss</span>
            </button>
          )}

          {/* Expand/Collapse Toggle Button */}
          <button
            type="button"
            onClick={toggleDrawerExpanded}
            className="p-1.5 rounded-md text-zinc-400 hover:text-zinc-100 hover:bg-white/[0.08] transition-colors cursor-pointer"
            title={isDrawerExpanded ? "Minimize Drawer" : "Expand Console"}
            aria-label={isDrawerExpanded ? "Minimize Drawer" : "Expand Console"}
          >
            {isDrawerExpanded ? (
              <ChevronDown className="w-4 h-4" />
            ) : (
              <ChevronUp className="w-4 h-4" />
            )}
          </button>
        </div>
      </div>

      {/* Expanded Console View */}
      {isDrawerExpanded && (
        <div className="flex-1 flex flex-col min-h-0 bg-[#08080a]">
          {error && (
            <div role="alert" className="mx-4 mt-3 rounded-lg border border-rose-500/30 bg-rose-950/30 px-3 py-2 text-sm text-rose-200 flex items-start justify-between gap-3">
              <div className="min-w-0">
                <div className="font-semibold">{stage === "recovery_required" ? "Recovery required" : isFailed ? "Transfer failed" : "Control request failed"}</div>
                <div className="mt-1 break-words text-rose-300/90 text-xs">{error}</div>
              </div>
              <button
                type="button"
                onClick={resetTransfer}
                className="shrink-0 px-2 py-1 rounded bg-rose-500/20 hover:bg-rose-500/30 text-rose-200 border border-rose-500/30 text-xs font-medium transition-colors cursor-pointer"
              >
                Dismiss
              </button>
            </div>
          )}
          {/* Subheader / Tabs Bar */}
          <div className="px-4 py-2 flex items-center justify-between border-b border-white/[0.06] bg-[#0a0a0e]">
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => setActiveTab("stream")}
                className={`flex items-center gap-1.5 px-3 py-1 rounded-md text-xs font-medium transition-colors cursor-pointer ${
                  activeTab === "stream"
                    ? "bg-[#161620] text-emerald-400 border border-white/[0.1]"
                    : "text-zinc-400 hover:text-zinc-200"
                }`}
              >
                <Activity className="w-3.5 h-3.5" />
                <span>Live Track Stream ({trackEvents.length})</span>
              </button>

              <button
                type="button"
                onClick={() => setActiveTab("logs")}
                className={`flex items-center gap-1.5 px-3 py-1 rounded-md text-xs font-medium transition-colors cursor-pointer ${
                  activeTab === "logs"
                    ? "bg-[#161620] text-emerald-400 border border-white/[0.1]"
                    : "text-zinc-400 hover:text-zinc-200"
                }`}
              >
                <Terminal className="w-3.5 h-3.5" />
                <span>Detailed Logs ({logs.length})</span>
              </button>
            </div>

            {/* Quick Metrics */}
            <div className="hidden sm:flex items-center gap-4 text-xs font-mono text-zinc-400 tabular-nums">
              <span className="text-emerald-400">
                Success: {progress?.successful ?? summary?.successful_tracks ?? 0}
              </span>
              <span className="text-rose-400">
                Failed: {progress?.failed ?? summary?.failed_tracks ?? 0}
              </span>
              <span className="text-zinc-500">
                Latency: {progress?.latency_ms ?? 0}ms
              </span>
            </div>
          </div>

          {/* Tab Content Panes */}
          <div className="flex-1 overflow-y-auto p-4 font-mono text-xs select-text">
            {activeTab === "stream" ? (
              <div className="space-y-1.5">
                {trackEvents.length === 0 ? (
                  <div className="text-zinc-600 text-center py-6">
                    Awaiting worker events...
                  </div>
                ) : (
                  trackEvents.map((ev, i) => (
                    <div
                      key={i}
                      className="flex items-center justify-between py-1 px-2.5 rounded bg-[#0f0f15] border border-white/[0.04]"
                    >
                      <div className="flex items-center gap-2 truncate">
                        <span
                          className={`w-1.5 h-1.5 rounded-full shrink-0 ${
                            ev.status >= 200 && ev.status < 300
                              ? "bg-emerald-400"
                              : "bg-rose-400"
                          }`}
                        />
                        <span className="text-zinc-200 truncate">{ev.track}</span>
                      </div>
                      <span className="text-[10px] text-zinc-500 font-mono tracking-wider shrink-0 ml-2">
                        HTTP {ev.status}
                      </span>
                    </div>
                  ))
                )}
              </div>
            ) : (
              <div className="space-y-1 text-zinc-300">
                {logs.length === 0 ? (
                  <div className="text-zinc-600 text-center py-6">No raw logs recorded.</div>
                ) : (
                  logs.map((log, i) => (
                    <div key={i} className="flex gap-2 text-[11px] leading-relaxed">
                      <span className="text-zinc-600 shrink-0">[{log.timestamp}]</span>
                      <span
                        className={
                          log.level === "error"
                            ? "text-rose-400"
                            : log.level === "warn"
                            ? "text-amber-400"
                            : "text-zinc-300"
                        }
                      >
                        {log.message}
                      </span>
                    </div>
                  ))
                )}
              </div>
            )}
          </div>

          {/* Audit Result / Completion Footer */}
          {auditResult && isCompleted && (
            <div className="px-4 py-2 border-t border-white/[0.06] bg-[#0c0c12] flex items-center justify-between text-xs">
              <div className="flex items-center gap-2 text-emerald-400">
                <ShieldCheck className="w-4 h-4 shrink-0" />
                <span>Integrity verified: {auditResult.total_found} tracks present on target.</span>
              </div>
              {onNavigateToHistory && (
                <button
                  type="button"
                  onClick={onNavigateToHistory}
                  className="text-zinc-400 hover:text-zinc-200 text-xs underline cursor-pointer"
                >
                  View in History
                </button>
              )}
            </div>
          )}
        </div>
      )}

      {/* Confirmation Modal for Cancel */}
      {showCancelPrompt && (
        <div className="fixed inset-0 z-50 bg-black/70 backdrop-blur-sm flex items-center justify-center p-4">
          <div className="bg-[#0c0c11] border border-white/[0.1] rounded-xl p-5 max-w-sm w-full shadow-2xl">
            <div className="flex items-center gap-2 text-amber-400 mb-2">
              <AlertTriangle className="w-5 h-5 shrink-0" />
              <h3 className="text-sm font-semibold text-zinc-100">Stop Migration?</h3>
            </div>
            <p className="text-xs text-zinc-400 mb-4 leading-relaxed">
              In-flight requests will be safely drained and checkpoints preserved. You can resume or rollback later from History.
            </p>
            <div className="flex items-center justify-end gap-2">
              <button
                type="button"
                onClick={() => setShowCancelPrompt(false)}
                className="px-3 py-1.5 rounded-lg text-xs font-medium bg-white/[0.06] hover:bg-white/[0.1] text-zinc-300 transition-colors cursor-pointer"
              >
                Continue Transfer
              </button>
              <button
                type="button"
                onClick={confirmCancel}
                className="px-3 py-1.5 rounded-lg text-xs font-medium bg-rose-500 hover:bg-rose-400 text-zinc-950 font-semibold transition-colors cursor-pointer"
              >
                Stop Transfer
              </button>
            </div>
          </div>
        </div>
      )}
    </aside>
  );
};
