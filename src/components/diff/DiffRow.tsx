import React from "react";
import { MatchResult } from "../../types/diff";
import { ArrowRight, AlertCircle, CheckCircle2, XCircle, Copy } from "lucide-react";

interface DiffRowProps {
  result: MatchResult;
  isSelected: boolean;
  isDuplicate?: boolean;
  onToggleSelect: (trackId: string) => void;
  onOpenDrawer?: (result: MatchResult) => void;
}

function formatDuration(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

function formatDelta(deltaMs: number): string {
  const deltaSec = Math.round(deltaMs / 1000);
  if (deltaSec === 0) return "0s";
  return deltaSec > 0 ? `+${deltaSec}s` : `${deltaSec}s`;
}

export const DiffRow: React.FC<DiffRowProps> = ({
  result,
  isSelected,
  isDuplicate = false,
  onToggleSelect,
  onOpenDrawer,
}) => {
  const { source_track, status, matched_track, candidates } = result;

  const topCandidate = candidates[0];
  const deltaMs = topCandidate?.duration_delta_ms ?? 0;

  const renderStatusBadge = () => {
    switch (status) {
      case "Exact":
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-medium bg-emerald-950/60 text-emerald-400 border border-emerald-500/30">
            <CheckCircle2 className="w-3 h-3 text-emerald-400" />
            <span>Exact</span>
          </span>
        );
      case "Ambiguous":
        return (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onOpenDrawer?.(result);
            }}
            className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-medium bg-amber-950/60 text-amber-400 border border-amber-500/30 hover:bg-amber-900/60 transition-colors cursor-pointer"
            title="Click to resolve ambiguity in drawer"
          >
            <AlertCircle className="w-3 h-3 text-amber-400" />
            <span>Amber ({formatDelta(deltaMs)})</span>
          </button>
        );
      case "NotFound":
        return (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onOpenDrawer?.(result);
            }}
            className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-medium bg-rose-950/60 text-rose-400 border border-rose-500/30 hover:bg-rose-900/60 transition-colors cursor-pointer"
            title="Click to manually find match in drawer"
          >
            <XCircle className="w-3 h-3 text-rose-400" />
            <span>Not Found</span>
          </button>
        );
      default:
        return null;
    }
  };

  return (
    <div
      className={`h-[56px] flex items-center px-4 border-b border-[#18181b] select-none transition-colors ${
        isSelected ? "bg-[#121215]" : "hover:bg-[#0c0c0e]"
      }`}
    >
      {/* Checkbox */}
      <div className="flex items-center mr-3">
        <input
          type="checkbox"
          checked={isSelected}
          onChange={() => onToggleSelect(source_track.id)}
          className="w-4 h-4 rounded border-zinc-700 bg-zinc-900 text-emerald-500 focus:ring-0 focus:ring-offset-0 cursor-pointer accent-emerald-500"
        />
      </div>

      {/* Source Track Column */}
      <div className="flex items-center min-w-0 flex-1 gap-3">
        {source_track.thumbnail_url ? (
          <img
            src={source_track.thumbnail_url}
            alt=""
            className="w-9 h-9 rounded object-cover flex-shrink-0 bg-zinc-900"
            loading="lazy"
          />
        ) : (
          <div className="w-9 h-9 rounded bg-zinc-800 flex-shrink-0 flex items-center justify-center text-[10px] text-zinc-500 font-mono">
            N/A
          </div>
        )}

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <span className="text-xs font-medium text-zinc-200 truncate">
              {source_track.title}
            </span>
            {source_track.is_explicit && (
              <span className="px-1 py-0.2 rounded bg-zinc-800 text-[9px] font-bold text-zinc-400">
                E
              </span>
            )}
            {isDuplicate && (
              <span className="inline-flex items-center gap-0.5 px-1.5 py-0.2 rounded bg-zinc-800 text-[10px] text-zinc-400 border border-zinc-700">
                <Copy className="w-2.5 h-2.5" />
                <span>Dup</span>
              </span>
            )}
          </div>
          <div className="text-[11px] text-zinc-500 truncate">
            {source_track.artists.join(", ")}
            <span className="mx-1 text-zinc-700">-</span>
            <span>{formatDuration(source_track.duration_ms)}</span>
          </div>
        </div>
      </div>

      {/* Visual Match Arrow & Status Pill */}
      <div className="flex items-center justify-center px-4 flex-shrink-0 gap-2">
        <ArrowRight className="w-3.5 h-3.5 text-zinc-600" />
        {renderStatusBadge()}
      </div>

      {/* Destination Matched Track Column */}
      <div className="flex items-center min-w-0 flex-1 gap-3 justify-end text-right">
        <div className="min-w-0 flex-1">
          {matched_track ? (
            <>
              <div className="flex items-center justify-end gap-1.5">
                <span className="text-xs font-medium text-zinc-200 truncate">
                  {matched_track.title}
                </span>
                {matched_track.is_explicit && (
                  <span className="px-1 py-0.2 rounded bg-zinc-800 text-[9px] font-bold text-zinc-400">
                    E
                  </span>
                )}
              </div>
              <div className="text-[11px] text-zinc-500 truncate">
                {matched_track.artists.join(", ")}
                <span className="mx-1 text-zinc-700">-</span>
                <span>{formatDuration(matched_track.duration_ms)}</span>
              </div>
            </>
          ) : (
            <span className="text-xs text-zinc-600 italic">No destination match</span>
          )}
        </div>

        {matched_track?.thumbnail_url ? (
          <img
            src={matched_track.thumbnail_url}
            alt=""
            className="w-9 h-9 rounded object-cover flex-shrink-0 bg-zinc-900"
            loading="lazy"
          />
        ) : (
          <div className="w-9 h-9 rounded bg-zinc-900 flex-shrink-0 flex items-center justify-center text-[10px] text-zinc-700 font-mono">
            {matched_track ? "N/A" : "-"}
          </div>
        )}
      </div>
    </div>
  );
};
