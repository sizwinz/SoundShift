import React, { useState } from "react";
import {
  ChevronUp,
  ChevronDown,
  Pause,
  Play,
  XSquare,
  ShieldCheck,
  AlertTriangle,
  RotateCcw,
  Terminal,
  Activity,
  ListMusic,
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
    trackEvents,
    isDrawerOpen,
    isDrawerExpanded,
    toggleDrawerExpanded,
    closeDrawer,
    pauseTransfer,
    resumeTransfer,
    cancelTransfer,
    retryFailedTracks,
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
  const isFailed = stage === "failed";
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
      className={`shrink-0 w-full z-30 bg-[#09090b] border-t border-[#27272a] shadow-2xl transition-all duration-300 flex flex-col ${
        isDrawerExpanded ? "h-[min(24rem,55dvh)]" : "h-14"
      }`}
      aria-label="Migration Telemetry Drawer"
    >
      {/* Top Thin Progress Line */}
      <div className="w-full h-1 bg-[#18181b] relative overflow-hidden">
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

      {/* Bar Header (Always Visible) */}
      <div className="h-13 px-4 flex items-center justify-between border-b border-[#27272a]/40 bg-[#0c0c0e]">
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
                  : "bg-emerald-400 animate-ping"
              }`}
            />
            <span className="text-xs font-bold text-zinc-100 uppercase tracking-wider font-mono">
              {stage}
            </span>
          </div>

          <div className="h-4 w-px bg-zinc-800 hidden sm:block" />

          {/* Current track or summary */}
          <div className="text-xs text-zinc-400 truncate max-w-xs sm:max-w-md">
            {progress?.current_track ? (
              <span className="text-zinc-200">{progress.current_track}</span>
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
        <div className="flex items-center gap-3 flex-shrink-0">
          <div className="text-xs font-mono font-medium text-zinc-300">
            <span>{processed}</span>
            <span className="text-zinc-600">/</span>
            <span>{total}</span>
            <span className="text-zinc-500 ml-1.5 font-normal">({percent}%)</span>
          </div>

          {/* Worker Controls */}
          {!isFinished && (
            <div className="flex items-center gap-1.5">
              {isPaused ? (
                <button
                  type="button"
                  onClick={resumeTransfer}
                  className="flex items-center gap-1 px-2.5 py-1 rounded bg-emerald-500/20 hover:bg-emerald-500/30 text-emerald-400 border border-emerald-500/30 text-xs font-medium transition-colors cursor-pointer"
                  title="Resume Transfer"
                >
                  <Play className="w-3 h-3 fill-current" />
                  <span className="hidden sm:inline">Resume</span>
                </button>
              ) : (
                <button
                  type="button"
                  onClick={pauseTransfer}
                  className="flex items-center gap-1 px-2.5 py-1 rounded bg-zinc-800 hover:bg-zinc-700 text-zinc-300 border border-zinc-700 text-xs font-medium transition-colors cursor-pointer"
                  title="Pause Transfer"
                >
                  <Pause className="w-3 h-3" />
                  <span className="hidden sm:inline">Pause</span>
                </button>
              )}

              <button
                type="button"
                onClick={handleCancelClick}
                className="flex items-center gap-1 px-2.5 py-1 rounded bg-zinc-900 hover:bg-rose-950/40 text-zinc-400 hover:text-rose-400 border border-zinc-800 text-xs font-medium transition-colors cursor-pointer"
                title="Cancel Transfer"
              >
                <XSquare className="w-3 h-3" />
                <span className="hidden sm:inline">Cancel</span>
              </button>
            </div>
          )}

          {/* Expand/Collapse Toggle Button per D-03 */}
          <button
            type="button"
            onClick={toggleDrawerExpanded}
            className="p-1.5 rounded-md text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors cursor-pointer"
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
        <div className="flex-1 flex flex-col min-h-0 bg-[#09090b]">
          {isFailed && error && (
            <div className="mx-4 mt-3 rounded-lg border border-rose-500/30 bg-rose-950/30 px-3 py-2 text-sm text-rose-200">
              <div className="font-semibold">Transfer failed</div>
              <div className="mt-1 break-words text-rose-300/90">{error}</div>
            </div>
          )}
          {/* Subheader / Tabs Bar */}
          <div className="px-4 py-2 flex items-center justify-between border-b border-[#27272a] bg-[#09090b]">
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => setActiveTab("stream")}
                className={`flex items-center gap-1.5 px-3 py-1 rounded text-xs font-medium transition-colors cursor-pointer ${
                  activeTab === "stream"
                    ? "bg-[#18181b] text-emerald-400 border border-[#27272a]"
                    : "text-zinc-400 hover:text-zinc-200"
                }`}
              >
                <Activity className="w-3.5 h-3.5" />
                <span>Live Track Stream ({trackEvents.length})</span>
              </button>

              <button
                type="button"
                onClick={() => setActiveTab("logs")}
                className={`flex items-center gap-1.5 px-3 py-1 rounded text-xs font-medium transition-colors cursor-pointer ${
                  activeTab === "logs"
                    ? "bg-[#18181b] text-emerald-400 border border-[#27272a]"
                    : "text-zinc-400 hover:text-zinc-200"
                }`}
              >
                <Terminal className="w-3.5 h-3.5" />
                <span>Detailed Logs ({logs.length})</span>
              </button>
            </div>

            {/* Quick Metrics */}
            <div className="hidden sm:flex items-center gap-4 text-xs font-mono text-zinc-400">
              <span className="text-emerald-400">
                Success: {progress?.successful ?? summary?.successful_tracks ?? 0}
              </span>
              <span className="text-rose-400">
                Failed: {progress?.failed ?? summary?.failed_tracks ?? 0}
              </span>
            </div>
          </div>

          {/* Tab 1: Live Track Stream per D-04 */}
          {activeTab === "stream" && (
            <div className="flex-1 overflow-y-auto p-3 space-y-1.5 font-mono text-xs">
              {trackEvents.length === 0 ? (
                <div className="h-full flex items-center justify-center text-zinc-600 text-xs">
                  Awaiting worker task dispatches...
                </div>
              ) : (
                trackEvents.map((evt) => (
                  <div
                    key={evt.id}
                    className="flex items-center justify-between p-2 rounded bg-[#0c0c0e] border border-zinc-900 hover:border-zinc-800 transition-colors"
                  >
                    <div className="flex items-center gap-3 truncate mr-2">
                      <span className="px-1.5 py-0.5 rounded bg-zinc-900 border border-zinc-800 text-[10px] text-zinc-400">
                        Worker {evt.workerId}
                      </span>
                      <span className="text-zinc-200 truncate">{evt.track}</span>
                    </div>

                    <div className="flex items-center gap-3 flex-shrink-0 text-[11px]">
                      <span className="text-zinc-500">{evt.latencyMs}ms</span>
                      <span
                        className={`px-1.5 py-0.5 rounded text-[10px] font-bold ${
                          evt.status === 200
                            ? "bg-emerald-950/60 text-emerald-400 border border-emerald-500/30"
                            : evt.status === 429
                            ? "bg-amber-950/60 text-amber-400 border border-amber-500/30"
                            : "bg-rose-950/60 text-rose-400 border border-rose-500/30"
                        }`}
                      >
                        HTTP {evt.status}
                      </span>
                    </div>
                  </div>
                ))
              )}
            </div>
          )}

          {/* Tab 2: Detailed Raw Logs per D-04 */}
          {activeTab === "logs" && (
            <div className="flex-1 overflow-y-auto p-3 space-y-1 font-mono text-[11px] bg-black/60 select-text">
              {logs.length === 0 ? (
                <div className="h-full flex items-center justify-center text-zinc-600">
                  No log entries recorded yet.
                </div>
              ) : (
                logs.map((log, idx) => (
                  <div
                    key={idx}
                    className={`flex items-start gap-2 py-0.5 ${
                      log.level === "error"
                        ? "text-rose-400"
                        : log.level === "warn"
                        ? "text-amber-400"
                        : "text-zinc-400"
                    }`}
                  >
                    <span className="text-zinc-600 flex-shrink-0">
                      [{new Date(log.timestamp * 1000).toLocaleTimeString()}]
                    </span>
                    {log.worker_id > 0 && (
                      <span className="text-zinc-500 flex-shrink-0">
                        [W{log.worker_id}]
                      </span>
                    )}
                    <span className="break-all">{log.message}</span>
                  </div>
                ))
              )}
            </div>
          )}

          {/* Completion / Cancellation Card per D-05, D-08 */}
          {isFinished && (
            <div className="p-3 bg-[#0c0c0e] border-t border-[#27272a] flex items-center justify-between">
              <div className="flex items-center gap-3">
                {auditResult?.is_verified ? (
                  <div className="flex items-center gap-1.5 text-xs text-emerald-400 font-semibold bg-emerald-950/40 border border-emerald-500/30 px-2.5 py-1 rounded">
                    <ShieldCheck className="w-4 h-4 text-emerald-400" />
                    <span>Audit Verified: All {auditResult.total_expected} songs matched</span>
                  </div>
                ) : auditResult ? (
                  <div className="flex items-center gap-1.5 text-xs text-amber-400 font-semibold bg-amber-950/40 border border-amber-500/30 px-2.5 py-1 rounded">
                    <AlertTriangle className="w-4 h-4 text-amber-400" />
                    <span>Audit Discrepancy: {auditResult.missing_ids.length} songs missing</span>
                  </div>
                ) : null}

                <div className="text-xs text-zinc-400 hidden md:block">
                  Snapshot: <span className="font-mono text-zinc-300">{summary?.snapshot_id || "Recorded"}</span>
                </div>
              </div>

              <div className="flex items-center gap-2">
                {/* Retry Failed Tracks per D-05 */}
                {summary && summary.failed_tracks > 0 && (
                  <button
                    type="button"
                    onClick={retryFailedTracks}
                    className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-amber-500 hover:bg-amber-400 text-black font-semibold text-xs transition-colors cursor-pointer"
                  >
                    <RotateCcw className="w-3.5 h-3.5" />
                    <span>Retry Failed Tracks ({summary.failed_tracks})</span>
                  </button>
                )}

                {onNavigateToHistory && (
                  <button
                    type="button"
                    onClick={() => {
                      closeDrawer();
                      onNavigateToHistory();
                    }}
                    className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-zinc-800 hover:bg-zinc-700 text-zinc-200 text-xs font-medium transition-colors cursor-pointer"
                  >
                    <ListMusic className="w-3.5 h-3.5" />
                    <span>View in History</span>
                  </button>
                )}

                <button
                  type="button"
                  onClick={resetTransfer}
                  className="px-3 py-1.5 rounded bg-zinc-900 hover:bg-zinc-800 border border-zinc-700 text-zinc-300 text-xs font-medium transition-colors cursor-pointer"
                >
                  Dismiss
                </button>
              </div>
            </div>
          )}
        </div>
      )}

      {/* Cancellation Prompt Dialog per D-06 */}
      {showCancelPrompt && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/80 backdrop-blur-xs">
          <div className="w-full max-w-sm bg-[#09090b] border border-amber-500/40 rounded-xl p-5 shadow-2xl space-y-4">
            <div className="flex items-center gap-2.5 text-amber-400">
              <AlertTriangle className="w-5 h-5 flex-shrink-0" />
              <h3 className="text-sm font-bold text-zinc-100">Cancel In-Flight Transfer?</h3>
            </div>

            <p className="text-xs text-zinc-400 leading-relaxed">
              New track processing will stop immediately. Songs already transferred ({progress?.successful ?? 0} tracks) will remain on the destination playlist, and you can roll them back at any time from the Transfer History view.
            </p>

            <div className="flex items-center justify-end gap-2 pt-2">
              <button
                type="button"
                onClick={() => setShowCancelPrompt(false)}
                className="px-3 py-1.5 rounded text-xs font-medium text-zinc-400 hover:text-zinc-200 transition-colors cursor-pointer"
              >
                Continue Transfer
              </button>

              <button
                type="button"
                onClick={confirmCancel}
                className="px-3.5 py-1.5 rounded bg-rose-600 hover:bg-rose-500 text-white font-semibold text-xs transition-colors cursor-pointer"
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
