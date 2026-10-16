import React from "react";
import { MatchCandidate } from "../../types/diff";
import { SourceTrack } from "../../types/provider";
import { AudioPlayButton } from "./AudioPlayButton";
import { Check, Clock, Disc } from "lucide-react";
import { Button } from "../ui";

interface CandidateCardProps {
  candidate: MatchCandidate;
  rank: number;
  isSelected?: boolean;
  onSelect: (track: SourceTrack) => void;
  audioSlot?: React.ReactNode;
}

function formatDuration(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

function formatDelta(deltaMs: number): string {
  const deltaSec = Math.round(deltaMs / 1000);
  if (deltaSec === 0) return "Exact length";
  return deltaSec > 0 ? `+${deltaSec}s` : `${deltaSec}s`;
}

export const CandidateCard: React.FC<CandidateCardProps> = ({
  candidate,
  rank,
  isSelected = false,
  onSelect,
  audioSlot,
}) => {
  const { track, similarity, duration_delta_ms } = candidate;
  const similarityPct = Math.round(similarity * 100);
  const isHighMatch = similarityPct >= 85;

  return (
    <div
      className={`p-3.5 rounded-xl border transition-all ${
        isSelected
          ? "bg-[#14141c] border-emerald-500/60 shadow-md ring-1 ring-emerald-500/30"
          : "bg-[#0b0b0f] border-white/[0.08] hover:border-white/[0.16] hover:bg-[#101016]"
      }`}
    >
      {/* Top Header: Rank & Badges */}
      <div className="flex items-center justify-between mb-2.5">
        <div className="flex items-center gap-1.5">
          <span className="px-2 py-0.5 rounded text-[10px] font-bold uppercase tracking-wider bg-white/[0.06] text-zinc-300 border border-white/[0.1]">
            {rank === 1 ? "#1 Top Match" : `#${rank}`}
          </span>
          {track.is_explicit && (
            <span className="px-1.5 py-0.2 rounded bg-white/[0.06] text-[9px] font-bold text-zinc-400">
              EXPLICIT
            </span>
          )}
        </div>

        <div className="flex items-center gap-2">
          {/* Duration Delta */}
          <span className="inline-flex items-center gap-1 text-[11px] text-zinc-400 font-mono tabular-nums">
            <Clock className="w-3 h-3 text-zinc-500" />
            <span>{formatDelta(duration_delta_ms)}</span>
          </span>

          {/* Similarity Badge */}
          <span
            className={`px-2 py-0.5 rounded text-[11px] font-mono font-semibold border tabular-nums ${
              isHighMatch
                ? "bg-emerald-950/60 text-emerald-400 border-emerald-500/30"
                : "bg-amber-950/60 text-amber-400 border-amber-500/30"
            }`}
          >
            {similarityPct}% match
          </span>
        </div>
      </div>

      {/* Main Metadata Row */}
      <div className="flex items-center gap-3">
        {track.thumbnail_url ? (
          <img
            src={track.thumbnail_url}
            alt=""
            className="w-12 h-12 rounded-lg object-cover shrink-0 bg-zinc-900 border border-white/[0.06]"
            loading="lazy"
          />
        ) : (
          <div className="w-12 h-12 rounded-lg bg-zinc-900 border border-white/[0.08] shrink-0 flex items-center justify-center text-zinc-500">
            <Disc className="w-5 h-5" />
          </div>
        )}

        <div className="min-w-0 flex-1">
          <div className="text-xs font-semibold text-zinc-100 truncate">
            {track.title}
          </div>
          <div className="text-[11px] text-zinc-400 truncate mt-0.5">
            {track.artists.join(", ")}
          </div>
          <div className="text-[10px] text-zinc-500 truncate mt-0.5 font-mono">
            <span>{track.album || "Single"}</span>
            <span className="mx-1.5 text-zinc-700">•</span>
            <span className="tabular-nums">{formatDuration(track.duration_ms)}</span>
          </div>
        </div>

        {/* Audio Slot or AudioPlayButton */}
        <div className="shrink-0">
          {audioSlot ?? <AudioPlayButton track={track} size="sm" />}
        </div>

        {/* Select Candidate Button */}
        <Button
          variant={isSelected ? "primary" : "secondary"}
          size="sm"
          onClick={() => onSelect(track)}
          className="shrink-0 ml-1"
        >
          {isSelected ? (
            <>
              <Check className="w-3.5 h-3.5" />
              <span>Selected</span>
            </>
          ) : (
            <span>Use Track</span>
          )}
        </Button>
      </div>
    </div>
  );
};
