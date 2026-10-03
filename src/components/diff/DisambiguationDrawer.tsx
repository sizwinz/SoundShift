import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { MatchResult, MatchCandidate } from "../../types/diff";
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
  CheckCircle2,
} from "lucide-react";

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
  // Manual search state
  const [searchQuery, setSearchQuery] = useState("");
  const [isSearching, setIsSearching] = useState(false);
  const [searchResults, setSearchResults] = useState<SourceTrack[]>([]);
  const [searchError, setSearchError] = useState<string | null>(null);

  // Direct URL paste state
  const [urlInput, setUrlInput] = useState("");
  const [isResolvingUrl, setIsResolvingUrl] = useState(false);
  const [urlError, setUrlError] = useState<string | null>(null);

  // Reset state when active matchResult changes
  useEffect(() => {
    if (matchResult) {
      setSearchQuery(`${matchResult.source_track.title} ${matchResult.source_track.artists[0] || ""}`);
      setSearchResults([]);
      setSearchError(null);
      setUrlInput("");
      setUrlError(null);
    }
  }, [matchResult]);

  // Handle Escape key to close
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

  // Execute custom manual query search per DIFF-05 and D-07
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

  // Execute direct URL resolution per DIFF-05 and D-07
  const handleResolveUrl = async (e?: React.FormEvent) => {
    if (e) e.preventDefault();
    const url = urlInput.trim();
    if (!url) return;

    setIsResolvingUrl(true);
    setUrlError(null);

    try {
      const resolved = await invoke<SourceTrack>("resolve_track_by_url", {
        service: targetService,
        url,
      });
      onResolve(source_track.id, resolved);
      onClose();
    } catch (err: unknown) {
      setUrlError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsResolvingUrl(false);
    }
  };

  const handleSelectCandidate = (track: SourceTrack) => {
    onResolve(source_track.id, track);
    onClose();
  };

  return (
    <>
      {/* Dimmed Backdrop per D-05 */}
      <div
        onClick={onClose}
        className="fixed inset-0 bg-black/60 backdrop-blur-xs z-40 transition-opacity animate-in fade-in duration-200"
      />

      {/* 500px Slide-Over Right Panel per D-05 */}
      <div
        className="fixed inset-y-0 right-0 w-full sm:w-[500px] z-50 bg-[#09090b] border-l border-[#27272a] shadow-2xl flex flex-col overflow-hidden animate-in slide-in-from-right duration-200"
        role="dialog"
        aria-modal="true"
        aria-labelledby="drawer-title"
      >
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-[#27272a] bg-[#0c0c0e]">
          <div>
            <h2 id="drawer-title" className="text-sm font-bold text-zinc-100 flex items-center gap-2">
              <span>Disambiguation Review</span>
              <span className="text-[10px] uppercase font-mono px-1.5 py-0.2 rounded bg-amber-950/70 text-amber-400 border border-amber-500/30">
                {matchResult.status}
              </span>
            </h2>
            <p className="text-[11px] text-zinc-500 mt-0.5">
              Compare source metadata against candidate recommendations or provide a custom match.
            </p>
          </div>

          <button
            type="button"
            onClick={onClose}
            className="p-1.5 rounded-lg text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors cursor-pointer"
            aria-label="Close drawer"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Scrollable Content Viewport */}
        <div className="flex-1 overflow-y-auto p-5 space-y-5 scrollbar-thin scrollbar-thumb-zinc-800">
          {/* Section 1: Source Track Card */}
          <div>
            <div className="text-[11px] font-semibold uppercase tracking-wider text-zinc-400 mb-2">
              Original Source Track
            </div>

            <div className="p-3.5 rounded-lg bg-[#121215] border border-zinc-800">
              <div className="flex items-start gap-3">
                {source_track.thumbnail_url ? (
                  <img
                    src={source_track.thumbnail_url}
                    alt=""
                    className="w-14 h-14 rounded object-cover flex-shrink-0 bg-zinc-900 border border-zinc-700"
                  />
                ) : (
                  <div className="w-14 h-14 rounded bg-zinc-800 flex-shrink-0 flex items-center justify-center text-zinc-500">
                    <Disc className="w-6 h-6" />
                  </div>
                )}

                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <span className="text-sm font-semibold text-zinc-100 truncate">
                      {source_track.title}
                    </span>
                    {source_track.is_explicit && (
                      <span className="px-1 py-0.2 rounded bg-zinc-800 text-[9px] font-bold text-zinc-400">
                        E
                      </span>
                    )}
                  </div>
                  <div className="text-xs text-zinc-400 truncate mt-0.5">
                    {source_track.artists.join(", ")}
                  </div>
                  <div className="text-[11px] text-zinc-500 truncate mt-1 flex items-center gap-2">
                    <span className="flex items-center gap-1">
                      <Clock className="w-3 h-3" />
                      {formatDuration(source_track.duration_ms)}
                    </span>
                    {source_track.album && (
                      <>
                        <span>•</span>
                        <span className="truncate">{source_track.album}</span>
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

                {/* Source Audio Preview per DIFF-04 */}
                <div className="flex-shrink-0 pt-1">
                  <AudioPlayButton track={source_track} size="sm" />
                </div>
              </div>
            </div>
          </div>

          {/* Section 2: Top Candidates */}
          <div>
            <div className="flex items-center justify-between mb-2">
              <span className="text-[11px] font-semibold uppercase tracking-wider text-zinc-400">
                Top Algorithmic Matches (Target: {targetService})
              </span>
              <span className="text-[10px] text-zinc-500 font-mono">
                {candidates.length} candidates found
              </span>
            </div>

            {candidates.length === 0 ? (
              <div className="p-4 rounded-lg bg-[#0c0c0e] border border-dashed border-[#27272a] text-center text-zinc-500 text-xs">
                No automatic candidates scored above threshold. Use manual search or paste a direct track URL below.
              </div>
            ) : (
              <div className="space-y-2.5">
                {candidates.slice(0, 3).map((candidate, idx) => {
                  const isCurrentMatch = matched_track?.id === candidate.track.id;
                  return (
                    <CandidateCard
                      key={candidate.track.id}
                      candidate={candidate}
                      rank={idx + 1}
                      isSelected={isCurrentMatch}
                      onSelect={handleSelectCandidate}
                    />
                  );
                })}
              </div>
            )}
          </div>

          {/* Section 3: Manual Search Override per DIFF-05 and D-07 */}
          <div>
            <div className="text-[11px] font-semibold uppercase tracking-wider text-zinc-400 mb-2">
              Manual Search Override
            </div>

            <form onSubmit={handleExecuteSearch} className="flex gap-2">
              <div className="relative flex-1">
                <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-zinc-500 pointer-events-none" />
                <input
                  type="text"
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  placeholder={`Search ${targetService} catalogue...`}
                  className="w-full bg-[#121215] border border-[#27272a] rounded pl-8 pr-3 py-1.5 text-xs text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-zinc-500 transition-colors"
                />
              </div>

              <button
                type="submit"
                disabled={isSearching || !searchQuery.trim()}
                className="px-3.5 py-1.5 rounded bg-zinc-800 hover:bg-zinc-700 text-zinc-200 text-xs font-medium border border-zinc-700 transition-colors cursor-pointer disabled:opacity-50 flex items-center gap-1.5"
              >
                {isSearching ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : "Search"}
              </button>
            </form>

            {searchError && (
              <div className="text-[11px] text-rose-400 mt-2 flex items-center gap-1">
                <AlertCircle className="w-3.5 h-3.5 flex-shrink-0" />
                <span>{searchError}</span>
              </div>
            )}

            {/* Custom Search Results List */}
            {searchResults.length > 0 && (
              <div className="mt-3 space-y-2">
                <div className="text-[10px] uppercase font-mono text-zinc-500">
                  Search Results ({searchResults.length})
                </div>
                {searchResults.map((track) => {
                  const candidate: MatchCandidate = {
                    track,
                    similarity: 0.9,
                    duration_delta_ms: Math.abs(track.duration_ms - source_track.duration_ms),
                  };
                  return (
                    <CandidateCard
                      key={track.id}
                      candidate={candidate}
                      rank={0}
                      isSelected={matched_track?.id === track.id}
                      onSelect={handleSelectCandidate}
                    />
                  );
                })}
              </div>
            )}
          </div>

          {/* Section 4: Direct URL / URI Paste Override per DIFF-05 and D-07 */}
          <div>
            <div className="text-[11px] font-semibold uppercase tracking-wider text-zinc-400 mb-2">
              Direct Track URL or URI Override
            </div>

            <form onSubmit={handleResolveUrl} className="flex gap-2">
              <div className="relative flex-1">
                <Link className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-zinc-500 pointer-events-none" />
                <input
                  type="text"
                  value={urlInput}
                  onChange={(e) => setUrlInput(e.target.value)}
                  placeholder={`Paste ${targetService} song link or track ID...`}
                  className="w-full bg-[#121215] border border-[#27272a] rounded pl-8 pr-3 py-1.5 text-xs text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-zinc-500 transition-colors"
                />
              </div>

              <button
                type="submit"
                disabled={isResolvingUrl || !urlInput.trim()}
                className="px-3.5 py-1.5 rounded bg-emerald-500 hover:bg-emerald-400 text-black text-xs font-semibold transition-colors cursor-pointer disabled:opacity-50 flex items-center gap-1.5"
              >
                {isResolvingUrl ? (
                  <Loader2 className="w-3.5 h-3.5 animate-spin" />
                ) : (
                  <>
                    <CheckCircle2 className="w-3.5 h-3.5" />
                    <span>Link</span>
                  </>
                )}
              </button>
            </form>

            {urlError && (
              <div className="text-[11px] text-rose-400 mt-2 flex items-center gap-1">
                <AlertCircle className="w-3.5 h-3.5 flex-shrink-0" />
                <span>{urlError}</span>
              </div>
            )}
          </div>
        </div>
      </div>
    </>
  );
};
