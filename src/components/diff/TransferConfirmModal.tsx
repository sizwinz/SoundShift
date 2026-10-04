import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { MatchResult } from "../../types/diff";
import { Playlist } from "../../types/provider";
import { isTauri } from "../../utils/tauri";
import {
  CheckCircle2,
  ShieldCheck,
  ArrowRight,
  X,
  Plus,
  FolderInput,
  Cpu,
  Search,
  Loader2,
  RefreshCw,
  Music,
  Check,
  Link2,
  ListMusic,
  AlertCircle,
} from "lucide-react";
import { defaultSettings, readSoundShiftSettings } from "../settings/SettingsPage";

export function parsePlaylistId(input: string): string {
  const trimmed = input.trim();
  if (!trimmed) return "";

  // Spotify URL: https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M
  const spotifyMatch = trimmed.match(/open\.spotify\.com\/playlist\/([a-zA-Z0-9]+)/);
  if (spotifyMatch) {
    return spotifyMatch[1];
  }

  // Spotify URI: spotify:playlist:37i9dQZF1DXcBWIGoYBM5M
  const spotifyUriMatch = trimmed.match(/spotify:playlist:([a-zA-Z0-9]+)/);
  if (spotifyUriMatch) {
    return spotifyUriMatch[1];
  }

  // YouTube / YouTube Music URL: https://music.youtube.com/playlist?list=PL...
  const ytMatch = trimmed.match(/[?&]list=([a-zA-Z0-9_-]+)/);
  if (ytMatch) {
    return ytMatch[1];
  }

  return trimmed;
}

export interface TransferTargetOptions {
  playlistName: string;
  isNewPlaylist: boolean;
  targetPlaylistId?: string | null;
  concurrency: number;
  skipDuplicates?: boolean;
}

interface TransferConfirmModalProps {
  isOpen: boolean;
  onClose: () => void;
  onConfirm: (options: TransferTargetOptions) => void;
  playlistTitle?: string;
  targetService: string;
  results: MatchResult[];
  selectedTrackIds: Set<string>;
}

export const TransferConfirmModal: React.FC<TransferConfirmModalProps> = ({
  isOpen,
  onClose,
  onConfirm,
  playlistTitle = "Playlist Transfer",
  targetService,
  results,
  selectedTrackIds,
}) => {
  const [isNewPlaylist, setIsNewPlaylist] = useState(true);
  const [customTitle, setCustomTitle] = useState(playlistTitle);
  const [targetPlaylistId, setTargetPlaylistId] = useState("");
  const [selectedPlaylist, setSelectedPlaylist] = useState<Playlist | null>(null);
  const [entryMode, setEntryMode] = useState<"select" | "manual">("select");
  const [manualInput, setManualInput] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [skipDuplicates, setSkipDuplicates] = useState(true);

  const [destinationPlaylists, setDestinationPlaylists] = useState<Playlist[]>([]);
  const [isLoadingPlaylists, setIsLoadingPlaylists] = useState(false);
  const [playlistError, setPlaylistError] = useState<string | null>(null);

  const [concurrency, setConcurrency] = useState(() => {
    try {
      return readSoundShiftSettings().concurrency;
    } catch {
      return defaultSettings.concurrency;
    }
  });

  const isYtMusic =
    targetService.toLowerCase().includes("ytmusic") ||
    targetService.toLowerCase().includes("youtube");
  const effectiveConcurrency = isYtMusic ? 1 : concurrency;

  const fetchDestinationPlaylists = async (forceRefresh: boolean = false) => {
    setIsLoadingPlaylists(true);
    setPlaylistError(null);

    if (!isTauri()) {
      setIsLoadingPlaylists(false);
      return;
    }

    try {
      const data = await invoke<Playlist[]>("list_provider_playlists", {
        service: targetService,
        refresh: forceRefresh,
      });
      setDestinationPlaylists(data);
      if (data.length > 0 && !targetPlaylistId) {
        setSelectedPlaylist(data[0]);
        setTargetPlaylistId(data[0].id);
      }
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setPlaylistError(msg);
    } finally {
      setIsLoadingPlaylists(false);
    }
  };

  useEffect(() => {
    if (isOpen && !isNewPlaylist && destinationPlaylists.length === 0 && !isLoadingPlaylists) {
      fetchDestinationPlaylists(false);
    }
  }, [isOpen, isNewPlaylist]);

  if (!isOpen) return null;

  const totalSelected = selectedTrackIds.size;
  const totalTracks = results.length;

  let exactSelected = 0;
  let manualResolved = 0;

  results.forEach((r) => {
    if (selectedTrackIds.has(r.source_track.id)) {
      if (r.match_method === "manual_disambiguation") {
        manualResolved++;
      } else {
        exactSelected++;
      }
    }
  });

  const skippedCount = totalTracks - totalSelected;

  const handleConfirm = () => {
    const resolvedTitle = isNewPlaylist
      ? (customTitle.trim() || playlistTitle)
      : (selectedPlaylist?.title || playlistTitle);

    onConfirm({
      playlistName: resolvedTitle,
      isNewPlaylist,
      targetPlaylistId: isNewPlaylist ? null : targetPlaylistId.trim() || null,
      concurrency: effectiveConcurrency,
      skipDuplicates: isNewPlaylist ? false : skipDuplicates,
    });
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/80 backdrop-blur-xs animate-in fade-in duration-150"
      role="dialog"
      aria-modal="true"
      aria-labelledby="confirm-modal-title"
    >
      <div className="w-full max-w-lg bg-[#09090b] border border-[#27272a] rounded-xl shadow-2xl overflow-hidden animate-in zoom-in-95 duration-150 max-h-[90vh] flex flex-col">
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-4 border-b border-[#27272a] bg-[#0c0c0e]">
          <div className="flex items-center gap-2.5">
            <div className="w-8 h-8 rounded-lg bg-emerald-950/60 border border-emerald-500/30 flex items-center justify-center text-emerald-400">
              <CheckCircle2 className="w-4 h-4" />
            </div>
            <div>
              <h2 id="confirm-modal-title" className="text-sm font-bold text-zinc-100">
                Confirm Playlist Transfer
              </h2>
              <p className="text-[11px] text-zinc-500">
                Configure destination playlist and concurrency before running batch migration.
              </p>
            </div>
          </div>

          <button
            type="button"
            onClick={onClose}
            className="p-1 rounded-md text-zinc-400 hover:text-zinc-100 hover:bg-zinc-800 transition-colors cursor-pointer"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Body Content */}
        <div className="p-5 space-y-4 overflow-y-auto flex-1">
          {/* Target Playlist Mode Selector per D-01 */}
          <div className="space-y-2">
            <label className="text-xs font-semibold text-zinc-300">
              Destination Target
            </label>
            <div className="grid grid-cols-2 gap-2">
              <button
                type="button"
                onClick={() => setIsNewPlaylist(true)}
                className={`p-3 rounded-lg border text-left flex flex-col gap-1 transition-colors cursor-pointer ${
                  isNewPlaylist
                    ? "bg-emerald-950/20 border-emerald-500/50 text-emerald-300"
                    : "bg-[#121215] border-zinc-800 text-zinc-400 hover:border-zinc-700"
                }`}
              >
                <div className="flex items-center gap-2 font-medium text-xs">
                  <Plus className="w-3.5 h-3.5 text-emerald-400" />
                  <span>Create New Playlist</span>
                </div>
                <div className="text-[10px] text-zinc-500">
                  Creates a fresh playlist on {targetService}
                </div>
              </button>

              <button
                type="button"
                onClick={() => setIsNewPlaylist(false)}
                className={`p-3 rounded-lg border text-left flex flex-col gap-1 transition-colors cursor-pointer ${
                  !isNewPlaylist
                    ? "bg-emerald-950/20 border-emerald-500/50 text-emerald-300"
                    : "bg-[#121215] border-zinc-800 text-zinc-400 hover:border-zinc-700"
                }`}
              >
                <div className="flex items-center gap-2 font-medium text-xs">
                  <FolderInput className="w-3.5 h-3.5 text-emerald-400" />
                  <span>Append to Existing</span>
                </div>
                <div className="text-[10px] text-zinc-500">
                  Appends tracks to an existing playlist
                </div>
              </button>
            </div>
          </div>

          {/* Conditional Input based on Mode */}
          {isNewPlaylist ? (
            <div className="space-y-1.5">
              <label className="text-[11px] font-medium text-zinc-400">
                New Playlist Name (per D-02)
              </label>
              <input
                type="text"
                value={customTitle}
                onChange={(e) => setCustomTitle(e.target.value)}
                placeholder="Playlist name..."
                className="w-full px-3 py-2 bg-[#121215] border border-zinc-800 rounded-md text-xs text-zinc-200 focus:outline-none focus:border-emerald-500/50"
              />
            </div>
          ) : (
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <label className="text-[11px] font-medium text-zinc-400">
                  {entryMode === "select" ? "Destination Playlist" : "Destination Playlist ID or Link"}
                </label>
                <button
                  type="button"
                  onClick={() => {
                    const nextMode = entryMode === "select" ? "manual" : "select";
                    setEntryMode(nextMode);
                    if (nextMode === "select" && destinationPlaylists.length === 0) {
                      fetchDestinationPlaylists(false);
                    }
                  }}
                  className="text-[11px] text-zinc-400 hover:text-emerald-400 transition-colors cursor-pointer flex items-center gap-1"
                >
                  {entryMode === "select" ? (
                    <>
                      <Link2 className="w-3 h-3" />
                      <span>Enter ID manually</span>
                    </>
                  ) : (
                    <>
                      <ListMusic className="w-3 h-3" />
                      <span>Choose from playlists</span>
                    </>
                  )}
                </button>
              </div>

              {entryMode === "select" ? (
                <div className="space-y-2">
                  {/* Search and Refresh bar */}
                  <div className="flex items-center gap-1.5">
                    <div className="relative flex-1">
                      <Search className="w-3.5 h-3.5 text-zinc-500 absolute left-2.5 top-1/2 -translate-y-1/2" />
                      <input
                        type="text"
                        value={searchQuery}
                        onChange={(e) => setSearchQuery(e.target.value)}
                        placeholder={`Search ${targetService} playlists...`}
                        className="w-full pl-8 pr-3 py-1.5 bg-[#121215] border border-zinc-800 rounded-md text-xs text-zinc-200 focus:outline-none focus:border-emerald-500/50 placeholder:text-zinc-600"
                      />
                    </div>
                    <button
                      type="button"
                      onClick={() => fetchDestinationPlaylists(true)}
                      disabled={isLoadingPlaylists}
                      className="p-1.5 rounded-md bg-[#121215] border border-zinc-800 text-zinc-400 hover:text-zinc-200 hover:border-zinc-700 transition-colors cursor-pointer disabled:opacity-40"
                      title="Refresh destination playlists"
                    >
                      <RefreshCw className={`w-3.5 h-3.5 ${isLoadingPlaylists ? "animate-spin text-emerald-400" : ""}`} />
                    </button>
                  </div>

                  {/* Playlist List or Status */}
                  {isLoadingPlaylists && destinationPlaylists.length === 0 ? (
                    <div className="p-6 rounded-lg border border-zinc-800 bg-[#121215] flex flex-col items-center justify-center gap-2 text-zinc-400">
                      <Loader2 className="w-5 h-5 animate-spin text-emerald-400" />
                      <span className="text-xs">Loading playlists from {targetService}...</span>
                    </div>
                  ) : playlistError && destinationPlaylists.length === 0 ? (
                    <div className="p-3 rounded-lg border border-rose-500/30 bg-rose-950/20 text-xs text-rose-300 space-y-2">
                      <div className="flex items-center gap-1.5 font-medium">
                        <AlertCircle className="w-4 h-4 text-rose-400 shrink-0" />
                        <span>Could not load destination playlists</span>
                      </div>
                      <p className="text-[11px] text-zinc-400">{playlistError}</p>
                      <div className="flex items-center gap-2 pt-1">
                        <button
                          type="button"
                          onClick={() => fetchDestinationPlaylists(true)}
                          className="px-2.5 py-1 rounded bg-zinc-800 text-zinc-200 hover:bg-zinc-700 text-[11px] cursor-pointer"
                        >
                          Retry
                        </button>
                        <button
                          type="button"
                          onClick={() => setEntryMode("manual")}
                          className="px-2.5 py-1 rounded bg-emerald-500/20 text-emerald-300 hover:bg-emerald-500/30 text-[11px] cursor-pointer"
                        >
                          Enter Playlist ID
                        </button>
                      </div>
                    </div>
                  ) : destinationPlaylists.length === 0 ? (
                    <div className="p-4 rounded-lg border border-zinc-800 bg-[#121215] text-center text-xs text-zinc-400 space-y-2">
                      <p>No existing playlists found on {targetService}.</p>
                      <button
                        type="button"
                        onClick={() => setEntryMode("manual")}
                        className="text-xs text-emerald-400 hover:underline cursor-pointer"
                      >
                        Enter Playlist ID manually
                      </button>
                    </div>
                  ) : (
                    <div className="max-h-44 overflow-y-auto space-y-1.5 pr-1 scrollbar-thin scrollbar-thumb-zinc-800">
                      {destinationPlaylists
                        .filter((p) =>
                          p.title.toLowerCase().includes(searchQuery.toLowerCase())
                        )
                        .map((p) => {
                          const isSelected = selectedPlaylist?.id === p.id || targetPlaylistId === p.id;
                          return (
                            <button
                              key={p.id}
                              type="button"
                              onClick={() => {
                                setSelectedPlaylist(p);
                                setTargetPlaylistId(p.id);
                              }}
                              className={`w-full p-2 rounded-lg border text-left flex items-center justify-between gap-2.5 transition-colors cursor-pointer ${
                                isSelected
                                  ? "bg-emerald-950/30 border-emerald-500/60 text-zinc-100"
                                  : "bg-[#121215] border-zinc-800/80 text-zinc-300 hover:border-zinc-700 hover:bg-[#16161a]"
                              }`}
                            >
                              <div className="flex items-center gap-2.5 min-w-0">
                                {p.cover_url ? (
                                  <img
                                    src={p.cover_url}
                                    alt={p.title}
                                    className="w-8 h-8 rounded object-cover shrink-0 bg-zinc-800"
                                  />
                                ) : (
                                  <div className="w-8 h-8 rounded bg-zinc-800 border border-zinc-700/50 flex items-center justify-center shrink-0 text-zinc-500">
                                    <Music className="w-4 h-4" />
                                  </div>
                                )}
                                <div className="min-w-0">
                                  <div className="text-xs font-medium truncate">{p.title}</div>
                                  <div className="text-[10px] text-zinc-500">
                                    {p.track_count} {p.track_count === 1 ? "track" : "tracks"}
                                  </div>
                                </div>
                              </div>
                              <div className="shrink-0">
                                {isSelected ? (
                                  <div className="w-4 h-4 rounded-full bg-emerald-500 flex items-center justify-center text-black">
                                    <Check className="w-2.5 h-2.5 stroke-[3]" />
                                  </div>
                                ) : (
                                  <div className="w-4 h-4 rounded-full border border-zinc-700" />
                                )}
                              </div>
                            </button>
                          );
                        })}
                    </div>
                  )}
                </div>
              ) : (
                <div className="space-y-1.5">
                  <input
                    type="text"
                    value={manualInput}
                    onChange={(e) => {
                      const val = e.target.value;
                      setManualInput(val);
                      setTargetPlaylistId(parsePlaylistId(val));
                    }}
                    placeholder="e.g. 37i9dQZF1DXcBWIGoYBM5M or playlist URL..."
                    className="w-full px-3 py-2 bg-[#121215] border border-zinc-800 rounded-md text-xs text-zinc-200 focus:outline-none focus:border-emerald-500/50"
                  />
                  {targetPlaylistId && manualInput !== targetPlaylistId && (
                    <p className="text-[10px] text-zinc-500 font-mono">
                      Parsed Playlist ID: <span className="text-emerald-400">{targetPlaylistId}</span>
                    </p>
                  )}
                  <p className="text-[10px] text-zinc-500">
                    Paste a Spotify playlist link/URI or YouTube Music playlist link/ID.
                  </p>
                </div>
              )}

              {/* Destination Deduplication Option */}
              <div className="pt-2.5 border-t border-zinc-800/80">
                <label className="flex items-start gap-2.5 cursor-pointer select-none">
                  <input
                    type="checkbox"
                    checked={skipDuplicates}
                    onChange={(e) => setSkipDuplicates(e.target.checked)}
                    className="mt-0.5 w-4 h-4 rounded border-zinc-700 bg-zinc-900 text-emerald-500 focus:ring-emerald-500 focus:ring-offset-0 cursor-pointer accent-emerald-500"
                  />
                  <div className="flex-1">
                    <div className="text-xs font-medium text-zinc-200">
                      Skip tracks already in destination playlist (deduplicate)
                    </div>
                    <p className="text-[11px] text-zinc-500 leading-relaxed">
                      Checks track ID, ISRC, and title/artist to avoid adding tracks that already exist in this playlist.
                    </p>
                  </div>
                </label>
              </div>
            </div>
          )}

          {/* Concurrency Selector per EXEC-02 */}
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <label className="text-xs font-semibold text-zinc-300 flex items-center gap-1.5">
                <Cpu className="w-3.5 h-3.5 text-zinc-400" />
                <span>Worker Concurrency</span>
              </label>
              <span className="text-xs font-mono text-emerald-400 font-semibold">
                {effectiveConcurrency} {effectiveConcurrency === 1 ? "worker" : "workers"}
              </span>
            </div>

            {isYtMusic ? (
              <div className="p-3 rounded-lg bg-zinc-900/70 border border-zinc-800 text-[11px] space-y-1">
                <div className="flex items-center gap-1.5 text-amber-400 font-medium">
                  <span>Single-Writer Mode Active</span>
                </div>
                <p className="text-[10px] text-zinc-400 leading-relaxed">
                  YouTube Music enforces strict playlist edit consistency. Mutations are serialized to 1 worker to eliminate 409 Conflict collisions and preserve playlist sequencing.
                </p>
              </div>
            ) : (
              <div className="flex items-center gap-2">
                {[1, 2, 4, 6, 8].map((c) => (
                  <button
                    key={c}
                    type="button"
                    onClick={() => setConcurrency(c)}
                    className={`flex-1 py-1.5 rounded text-xs font-medium border transition-colors cursor-pointer ${
                      concurrency === c
                        ? "bg-emerald-500 text-black border-emerald-400 font-bold"
                        : "bg-[#121215] border-zinc-800 text-zinc-400 hover:text-zinc-200 hover:border-zinc-700"
                    }`}
                  >
                    {c} {c === 4 && <span className="text-[9px] block text-inherit font-normal">(Default)</span>}
                  </button>
                ))}
              </div>
            )}
          </div>

          {/* Breakdown Stats */}
          <div className="grid grid-cols-3 gap-2 text-center pt-1">
            <div className="p-2.5 rounded-lg bg-[#0c0c0e] border border-zinc-800">
              <div className="text-base font-bold text-emerald-400 font-mono">
                {exactSelected}
              </div>
              <div className="text-[10px] text-zinc-500 uppercase mt-0.5">Exact Matches</div>
            </div>

            <div className="p-2.5 rounded-lg bg-[#0c0c0e] border border-zinc-800">
              <div className="text-base font-bold text-amber-400 font-mono">
                {manualResolved}
              </div>
              <div className="text-[10px] text-zinc-500 uppercase mt-0.5">Resolved</div>
            </div>

            <div className="p-2.5 rounded-lg bg-[#0c0c0e] border border-zinc-800">
              <div className="text-base font-bold text-zinc-400 font-mono">
                {skippedCount}
              </div>
              <div className="text-[10px] text-zinc-500 uppercase mt-0.5">Skipped</div>
            </div>
          </div>

          {/* Safety Notice per D-13 & SAFE-01 */}
          <div className="p-3 rounded-lg bg-emerald-950/30 border border-emerald-500/20 text-emerald-300 text-xs flex items-start gap-2.5">
            <ShieldCheck className="w-4 h-4 text-emerald-400 flex-shrink-0 mt-0.5" />
            <div className="text-[11px] leading-relaxed">
              <span className="font-semibold text-emerald-300">1-Click Snapshot Rollback: </span>
              SoundShift will transactionally write an immutable local snapshot to SQLite prior to mutations, allowing instant 1-click rollback at any time.
            </div>
          </div>
        </div>

        {/* Modal Action Buttons */}
        <div className="flex items-center justify-end gap-2.5 px-5 py-3.5 border-t border-[#27272a] bg-[#0c0c0e]">
          <button
            type="button"
            onClick={onClose}
            className="px-3.5 py-1.5 rounded text-xs font-medium text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800 transition-colors cursor-pointer"
          >
            Back to Review
          </button>

          <button
            type="button"
            onClick={handleConfirm}
            disabled={isNewPlaylist ? !customTitle.trim() : !targetPlaylistId.trim()}
            className="flex items-center gap-1.5 px-4 py-2 rounded bg-emerald-500 hover:bg-emerald-400 text-black font-semibold text-xs transition-colors cursor-pointer shadow-md disabled:opacity-40 disabled:cursor-not-allowed"
          >
            <span>Confirm & Start Transfer</span>
            <ArrowRight className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>
    </div>
  );
};
