import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { MatchResult } from "../../types/diff";
import { SourceTrack } from "../../types/provider";
import { CandidateCard } from "./CandidateCard";
import { AudioPlayButton } from "./AudioPlayButton";
import {
  X,
  Search,
  Link,
  Disc,
  Clock,
  Fingerprint,
  AlertCircle,
  Loader2,
} from "lucide-react";
import { Button, Card, Badge } from "../ui";

interface DisambiguationDrawerProps {
  matchResult: MatchResult | null;
  targetService: string;
  isOpen: boolean;
  onClose: () => void;
  onResolve: (sourceTrackId: string, resolvedTrack: SourceTrack) => void;
}

function formatDuration(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${minutes}:${seconds.toString().padStart(2, "0")}`;
}

export const DisambiguationDrawer: React.FC<DisambiguationDrawerProps> = ({
  matchResult,
  targetService,
  isOpen,
  onClose,
  onResolve,
}) => {
  const [searchQuery, setSearchQuery] = useState("");
  const [isSearching, setIsSearching] = useState(false);
  const [searchResults, setSearchResults] = useState<SourceTrack[]>([]);
  const [searchError, setSearchError] = useState<string | null>(null);

  const [urlInput, setUrlInput] = useState("");
  const [isResolvingUrl, setIsResolvingUrl] = useState(false);
  const [urlError, setUrlError] = useState<string | null>(null);

  useEffect(() => {
    if (matchResult) {
      setSearchQuery(`${matchResult.source_track.title} ${matchResult.source_track.artists[0] || ""}`);
      setSearchResults([]);
      setSearchError(null);
      setUrlInput("");
      setUrlError(null);
    }
  }, [matchResult]);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && isOpen) {
        onClose();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, onClose]);

  if (!isOpen || !matchResult) {
    return null;
  }

  const { source_track, candidates, matched_track } = matchResult;

  const handleExecuteSearch = async (e?: React.FormEvent) => {
    if (e) e.preventDefault();
    const query = searchQuery.trim();
    if (!query) return;

    setIsSearching(true);
    setSearchError(null);

    try {
      const results = await invoke<SourceTrack[]>("search_provider_tracks", {
        service: targetService,
        query,
      });
      setSearchResults(results);
      if (results.length === 0) {
        setSearchError(`No tracks found matching "${query}" on ${targetService}`);
      }
    } catch (err: unknown) {
      setSearchError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsSearching(false);
    }
  };

  const handleResolveUrl = async (e: React.FormEvent) => {
    e.preventDefault();
    const trimmedUrl = urlInput.trim();
    if (!trimmedUrl) return;

    setIsResolvingUrl(true);
    setUrlError(null);

    try {
      const resolved = await invoke<SourceTrack>("resolve_track_url", {
        service: targetService,
        url: trimmedUrl,
      });
      onResolve(source_track.id, resolved);
    } catch (err: unknown) {
      setUrlError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsResolvingUrl(false);
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="drawer-title"
      className="fixed inset-0 z-50 overflow-hidden bg-black/70 backdrop-blur-md animate-in fade-in duration-200"
    >
      <div className="absolute inset-0" onClick={onClose} />

      <aside className="absolute inset-y-0 right-0 max-w-full flex pl-10">
        <div className="w-screen max-w-xl bg-[#09090d] border-l border-white/[0.08] shadow-2xl flex flex-col justify-between">
          {/* Header */}
          <div className="p-5 border-b border-white/[0.06] bg-[#0c0c12]/80 flex items-start justify-between">
            <div>
              <h2 id="drawer-title" className="text-sm font-bold text-zinc-100 flex items-center gap-2">
                <span>Disambiguation Review</span>
                <Badge variant={matchResult.status === "Ambiguous" ? "amber" : "unmatched"}>
                  {matchResult.status}
                </Badge>
              </h2>
              <p className="text-[11px] text-zinc-400 mt-0.5">
                Compare source metadata against candidate recommendations or provide a custom match.
              </p>
            </div>

            <button
              type="button"
              onClick={onClose}
              className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-white/[0.06] transition-colors cursor-pointer"
              aria-label="Close drawer"
            >
              <X className="w-4 h-4" />
            </button>
          </div>

          {/* Scrollable Content Viewport */}
          <div className="flex-1 overflow-y-auto p-5 space-y-5">
            {/* Source Track Card */}
            <div>
              <div className="text-[10px] font-mono uppercase tracking-wider text-zinc-500 font-semibold mb-2">
                Original Source Track
              </div>

              <Card className="p-3.5 bg-[#0f0f15] border-white/[0.08]">
                <div className="flex items-start gap-3">
                  {source_track.thumbnail_url ? (
                    <img
                      src={source_track.thumbnail_url}
                      alt=""
                      className="w-14 h-14 rounded-lg object-cover shrink-0 bg-zinc-900 border border-white/[0.06]"
                    />
                  ) : (
                    <div className="w-14 h-14 rounded-lg bg-zinc-800 shrink-0 flex items-center justify-center text-zinc-500">
                      <Disc className="w-6 h-6" />
                    </div>
                  )}

                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5">
                      <span className="text-sm font-semibold text-zinc-100 truncate">
                        {source_track.title}
                      </span>
                      {source_track.is_explicit && (
                        <span className="px-1 py-0.2 rounded bg-white/[0.08] text-[9px] font-bold text-zinc-400">
                          E
                        </span>
                      )}
                    </div>
                    <div className="text-xs text-zinc-400 truncate mt-0.5">
                      {source_track.artists.join(", ")}
                    </div>
                    <div className="text-[11px] text-zinc-500 truncate mt-1 flex items-center gap-2 font-mono tabular-nums">
                      <span className="flex items-center gap-1">
                        <Clock className="w-3 h-3 text-zinc-500" />
                        {formatDuration(source_track.duration_ms)}
                      </span>
                      {source_track.album && (
                        <>
                          <span>•</span>
                          <span className="truncate font-sans">{source_track.album}</span>
                        </>
                      )}
                    </div>
                    {source_track.isrc && (
                      <div className="text-[10px] text-zinc-600 font-mono mt-1 flex items-center gap-1">
                        <Fingerprint className="w-3 h-3" />
                        <span>ISRC: {source_track.isrc}</span>
                      </div>
                    )}
                  </div>

                  <div className="shrink-0 pt-1">
                    <AudioPlayButton track={source_track} size="sm" />
                  </div>
                </div>
              </Card>
            </div>

            {/* Top Candidates */}
            <div>
              <div className="flex items-center justify-between mb-2">
                <span className="text-[10px] font-mono uppercase tracking-wider text-zinc-500 font-semibold">
                  Top Algorithmic Matches (Target: {targetService})
                </span>
                <span className="text-[10px] text-zinc-500 font-mono">
                  {candidates.length} candidates found
                </span>
              </div>

              {candidates.length === 0 ? (
                <div className="p-4 rounded-xl bg-[#0b0b0f] border border-dashed border-white/[0.08] text-center text-zinc-500 text-xs">
                  No automatic candidates scored above threshold. Use manual search or paste a direct track URL below.
                </div>
              ) : (
                <div className="space-y-2">
                  {candidates.map((cand, index) => (
                    <CandidateCard
                      key={cand.track.id}
                      candidate={cand}
                      rank={index + 1}
                      isSelected={matched_track?.id === cand.track.id}
                      onSelect={(track) => onResolve(source_track.id, track)}
                    />
                  ))}
                </div>
              )}
            </div>

            {/* Custom Search Form */}
            <div>
              <div className="text-[10px] font-mono uppercase tracking-wider text-zinc-500 font-semibold mb-2">
                Custom Manual Catalog Search
              </div>

              <form onSubmit={handleExecuteSearch} className="flex gap-2">
                <div className="relative flex-1">
                  <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-zinc-500" />
                  <input
                    type="text"
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                    placeholder={`Search song title or artist on ${targetService}...`}
                    className="w-full bg-[#121217] border border-white/[0.08] focus:border-zinc-500 rounded-lg pl-9 pr-3 py-1.5 text-xs text-zinc-200 placeholder:text-zinc-600 focus:outline-none"
                  />
                </div>
                <Button
                  type="submit"
                  variant="secondary"
                  size="sm"
                  disabled={isSearching || !searchQuery.trim()}
                >
                  {isSearching ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : "Search"}
                </Button>
              </form>

              {searchError && (
                <div className="mt-2 p-2.5 bg-rose-950/30 border border-rose-500/30 rounded-lg text-rose-300 text-xs flex items-center gap-2">
                  <AlertCircle className="w-3.5 h-3.5 shrink-0" />
                  <span className="text-[11px]">{searchError}</span>
                </div>
              )}

              {searchResults.length > 0 && (
                <div className="mt-3 space-y-2">
                  <span className="text-[11px] text-zinc-400 font-medium">Search Results ({searchResults.length}):</span>
                  {searchResults.map((t, idx) => (
                    <CandidateCard
                      key={t.id}
                      candidate={{
                        track: t,
                        similarity: 1.0,
                        duration_delta_ms: Math.abs(t.duration_ms - source_track.duration_ms),
                      }}
                      rank={idx + 1}
                      isSelected={matched_track?.id === t.id}
                      onSelect={(track) => onResolve(source_track.id, track)}
                    />
                  ))}
                </div>
              )}
            </div>

            {/* Direct URL Form */}
            <div>
              <div className="text-[10px] font-mono uppercase tracking-wider text-zinc-500 font-semibold mb-2">
                Direct Track URL Paste
              </div>

              <form onSubmit={handleResolveUrl} className="flex gap-2">
                <div className="relative flex-1">
                  <Link className="absolute left-3 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-zinc-500" />
                  <input
                    type="url"
                    value={urlInput}
                    onChange={(e) => setUrlInput(e.target.value)}
                    placeholder={`https://music.youtube.com/watch?v=... or https://open.spotify.com/track/...`}
                    className="w-full bg-[#121217] border border-white/[0.08] focus:border-zinc-500 rounded-lg pl-9 pr-3 py-1.5 text-xs text-zinc-200 placeholder:text-zinc-600 focus:outline-none"
                  />
                </div>
                <Button
                  type="submit"
                  variant="secondary"
                  size="sm"
                  disabled={isResolvingUrl || !urlInput.trim()}
                >
                  {isResolvingUrl ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : "Resolve"}
                </Button>
              </form>

              {urlError && (
                <div className="mt-2 p-2.5 bg-rose-950/30 border border-rose-500/30 rounded-lg text-rose-300 text-xs flex items-center gap-2">
                  <AlertCircle className="w-3.5 h-3.5 shrink-0" />
                  <span className="text-[11px]">{urlError}</span>
                </div>
              )}
            </div>
          </div>

          {/* Footer */}
          <div className="p-4 border-t border-white/[0.06] bg-[#0c0c12]/90 flex items-center justify-between">
            <span className="text-[11px] text-zinc-500">
              Resolved choices update staged matches immediately.
            </span>
            <Button variant="secondary" size="sm" onClick={onClose}>
              Done
            </Button>
          </div>
        </div>
      </aside>
    </div>
  );
};
