import React from "react";
import { SourceTrack } from "../../types/provider";
import { useAudioPreview } from "../../context/AudioContext";
import { Play, Pause, ExternalLink, VolumeX } from "lucide-react";

interface AudioPlayButtonProps {
  track: SourceTrack;
  size?: "sm" | "md";
  service?: string;
}

export const AudioPlayButton: React.FC<AudioPlayButtonProps> = ({
  track,
  size = "sm",
  service = "spotify",
}) => {
  const { activeTrackId, isPlaying, progress, playPreview } = useAudioPreview();

  const isCurrentTrack = activeTrackId === track.id;
  const isCurrentPlaying = isCurrentTrack && isPlaying;

  // External link URL for tracks without preview stream per D-10
  const externalUrl =
    service === "spotify"
      ? `https://open.spotify.com/track/${track.id}`
      : `https://music.youtube.com/watch?v=${track.id}`;

  if (!track.preview_url) {
    return (
      <a
        href={externalUrl}
        target="_blank"
        rel="noopener noreferrer"
        onClick={(e) => e.stopPropagation()}
        className="inline-flex items-center gap-1 px-2 py-1 rounded bg-zinc-900 hover:bg-zinc-800 border border-zinc-800 text-[10px] text-zinc-500 hover:text-zinc-300 transition-colors"
        title="Open in provider web player (no in-app preview stream available)"
      >
        <VolumeX className="w-3 h-3 text-zinc-600" />
        <span className="hidden sm:inline">No preview</span>
        <ExternalLink className="w-2.5 h-2.5" />
      </a>
    );
  }

  const dimension = size === "sm" ? 32 : 38;
  const strokeWidth = 2.5;
  const radius = (dimension - strokeWidth * 2) / 2;
  const circumference = 2 * Math.PI * radius;
  const strokeDashoffset = isCurrentTrack
    ? circumference - progress * circumference
    : circumference;

  return (
    <button
      type="button"
      onClick={(e) => {
        e.stopPropagation();
        playPreview(track.id, track.preview_url!);
      }}
      className="relative flex items-center justify-center rounded-full bg-zinc-900 hover:bg-zinc-800 text-zinc-300 hover:text-white transition-all cursor-pointer flex-shrink-0 group"
      style={{ width: dimension, height: dimension }}
      title={isCurrentPlaying ? "Pause preview" : "Play 30s preview"}
      aria-label={isCurrentPlaying ? "Pause preview" : "Play preview"}
    >
      {/* SVG Progress Ring */}
      <svg
        className="absolute inset-0 -rotate-90 pointer-events-none"
        width={dimension}
        height={dimension}
      >
        {/* Background track circle */}
        <circle
          cx={dimension / 2}
          cy={dimension / 2}
          r={radius}
          stroke="rgba(39, 39, 42, 0.6)"
          strokeWidth={strokeWidth}
          fill="none"
        />
        {/* Animated active progress circle */}
        {isCurrentTrack && (
          <circle
            cx={dimension / 2}
            cy={dimension / 2}
            r={radius}
            stroke="#22c55e"
            strokeWidth={strokeWidth}
            fill="none"
            strokeDasharray={circumference}
            strokeDashoffset={strokeDashoffset}
            strokeLinecap="round"
            className="transition-all duration-100 ease-linear"
          />
        )}
      </svg>

      {/* Play / Pause Icon */}
      {isCurrentPlaying ? (
        <Pause className="w-3.5 h-3.5 fill-emerald-400 text-emerald-400" />
      ) : (
        <Play className="w-3.5 h-3.5 fill-current ml-0.5 group-hover:scale-110 transition-transform" />
      )}
    </button>
  );
};
