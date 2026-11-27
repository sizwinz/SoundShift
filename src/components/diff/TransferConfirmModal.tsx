import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { MatchResult } from "../../types/diff";
import { Playlist } from "../../types/provider";
import { isTauri } from "../../utils/tauri";
import { isRowSelectable, occurrenceKey } from "./StagingTable";
import { Button, Modal, SearchInput, Badge } from "../ui";
import {
  CheckCircle2,
  ShieldCheck,
  ArrowRight,
  Plus,
  FolderInput,
  Loader2,
  RefreshCw,
  Music,
  Check,
  Link2,
  ListMusic,
  AlertCircle,
} from "lucide-react";

export function parsePlaylistId(input: string): string {
  const trimmed = input.trim();
  if (!trimmed) return "";

  const spotifyMatch = trimmed.match(/open\.spotify\.com\/playlist\/([a-zA-Z0-9]+)/);
  if (spotifyMatch) {
    return spotifyMatch[1];
  }

  const spotifyUriMatch = trimmed.match(/spotify:playlist:([a-zA-Z0-9]+)/);
  if (spotifyUriMatch) {
    return spotifyUriMatch[1];
  }

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

  const effectiveConcurrency = 1;

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

  const totalTracks = results.length;
  let exactSelected = 0;
  let manualResolved = 0;
  let selectedPlayableCount = 0;
  const seenTargetIds = new Set<string>();
  let duplicateTargetCount = 0;

  results.forEach((r, idx) => {
    const key = occurrenceKey(r, idx);
    const isSelected = selectedTrackIds.has(key) || selectedTrackIds.has(r.source_track.id);
    if (isSelected && isRowSelectable(r)) {
      selectedPlayableCount++;
      if (r.match_method === "manual_disambiguation") {
        manualResolved++;
      } else {
        exactSelected++;
      }
      if (r.matched_track?.id) {
        if (seenTargetIds.has(r.matched_track.id)) {
          duplicateTargetCount++;
        } else {
          seenTargetIds.add(r.matched_track.id);
        }
      }
    }
  });

  const totalSelected = selectedPlayableCount > 0 ? selectedPlayableCount : selectedTrackIds.size;
  const skippedCount = Math.max(0, totalTracks - totalSelected);
  const isYoutubeDuplicateConflict =
    targetService === "ytmusic" && duplicateTargetCount > 0 && !skipDuplicates;

  const handleConfirm = () => {
    const resolvedTitle = isNewPlaylist
      ? (customTitle.trim() || playlistTitle)
      : (selectedPlaylist?.title || playlistTitle);

    onConfirm({
      playlistName: resolvedTitle,
      isNewPlaylist,
      targetPlaylistId: isNewPlaylist ? null : targetPlaylistId.trim() || null,
      concurrency: effectiveConcurrency,
      skipDuplicates,
    });
  };

  const confirmDisabled =
    (isNewPlaylist ? !customTitle.trim() : !targetPlaylistId.trim()) ||
    isYoutubeDuplicateConflict;

  return (
    <Modal
      isOpen={isOpen}
      onClose={onClose}
      title="Confirm Playlist Transfer"
      description="Choose the destination playlist before starting the transfer."
      titleId="confirm-modal-title"
      maxWidth="max-w-lg"
      accent="emerald"
      icon={
        <div className="w-8 h-8 rounded-lg bg-emerald-950/60 border border-emerald-500/30 flex items-center justify-center text-emerald-400">
          <CheckCircle2 className="w-4 h-4" />
        </div>
      }
      footer={
        <>
          <Button variant="ghost" size="md" onClick={onClose}>
            Back to Review
          </Button>
          <Button
            variant="primary"
            size="md"
            onClick={handleConfirm}
            disabled={confirmDisabled}
          >
            <span>Confirm & Start Transfer</span>
            <ArrowRight className="w-3.5 h-3.5" />
          </Button>
        </>
      }
    >
      <div className="space-y-4">
        {/* Target Playlist Mode Selector */}
        <div className="space-y-2">
          <label className="text-xs font-semibold text-zinc-300 block">
            Destination Target
          </label>
          <div className="grid grid-cols-2 gap-2">
            <button
              type="button"
              onClick={() => setIsNewPlaylist(true)}
              className={`p-3 rounded-lg border text-left flex flex-col gap-1 transition-colors cursor-pointer ${
                isNewPlaylist
                  ? "bg-emerald-950/20 border-emerald-500/50 text-emerald-300"
                  : "bg-[#111116] border-white/[0.08] text-zinc-400 hover:border-white/[0.16]"
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
                  : "bg-[#111116] border-white/[0.08] text-zinc-400 hover:border-white/[0.16]"
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
            <label className="text-[11px] font-medium text-zinc-400 block">
              New Playlist Name
            </label>
            <input
              type="text"
              value={customTitle}
              onChange={(e) => setCustomTitle(e.target.value)}
              placeholder="Enter destination playlist name..."
              className="w-full px-3 py-2 bg-[#121217] border border-white/[0.08] rounded-lg text-xs text-zinc-200 focus:outline-none focus:border-emerald-500/60"
            />
          </div>
        ) : (
          <div className="space-y-2.5">
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
                <div className="flex items-center gap-1.5">
                  <SearchInput
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                    onClear={() => setSearchQuery("")}
                    placeholder={`Search ${targetService} playlists...`}
                    className="flex-1"
                  />
                  <Button
                    variant="secondary"
                    size="sm"
                    onClick={() => fetchDestinationPlaylists(true)}
                    disabled={isLoadingPlaylists}
                    title="Refresh destination playlists"
                  >
                    <RefreshCw className={`w-3.5 h-3.5 ${isLoadingPlaylists ? "animate-spin text-emerald-400" : ""}`} />
                  </Button>
                </div>

                {isLoadingPlaylists && destinationPlaylists.length === 0 ? (
                  <div className="p-6 rounded-lg border border-white/[0.08] bg-[#111116] flex flex-col items-center justify-center gap-2 text-zinc-400">
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
                      <Button variant="secondary" size="xs" onClick={() => fetchDestinationPlaylists(true)}>
                        Retry
                      </Button>
                      <Button variant="outline" size="xs" onClick={() => setEntryMode("manual")}>
                        Enter Playlist ID
                      </Button>
                    </div>
                  </div>
                ) : destinationPlaylists.length === 0 ? (
                  <div className="p-4 rounded-lg border border-white/[0.08] bg-[#111116] text-center text-xs text-zinc-400 space-y-2">
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
                                : "bg-[#111116] border-white/[0.08] text-zinc-300 hover:border-white/[0.16] hover:bg-[#16161d]"
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
                                <div className="w-8 h-8 rounded bg-zinc-900 border border-white/[0.08] flex items-center justify-center shrink-0 text-zinc-500">
                                  <Music className="w-4 h-4" />
                                </div>
                              )}
                              <div className="min-w-0">
                                <div className="text-xs font-medium truncate">{p.title}</div>
                                <div className="text-[10px] text-zinc-500 tabular-nums">
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
                  className="w-full px-3 py-2 bg-[#121217] border border-white/[0.08] rounded-lg text-xs text-zinc-200 focus:outline-none focus:border-emerald-500/60"
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
          </div>
        )}

        {/* Duplicate Handling */}
        <div className="p-3 rounded-lg bg-[#111116] border border-white/[0.08] space-y-2">
          <label className="flex items-start gap-2.5 cursor-pointer select-none">
            <input
              type="checkbox"
              checked={skipDuplicates}
              onChange={(e) => setSkipDuplicates(e.target.checked)}
              className="mt-0.5 w-4 h-4 rounded border-zinc-700 bg-zinc-900 text-emerald-500 focus:ring-emerald-500 focus:ring-offset-0 cursor-pointer accent-emerald-500"
            />
            <div className="flex-1 space-y-0.5">
              <div className="text-xs font-medium text-zinc-200">
                {isNewPlaylist
                  ? "Skip duplicate destination tracks"
                  : "Skip tracks already in destination playlist (deduplicate)"}
              </div>
              <p className="text-[11px] text-zinc-500 leading-relaxed">
                {isNewPlaylist
                  ? "Ensures each matched song is added once when distinct source tracks resolve to the same destination track ID."
                  : "Checks track ID, ISRC, and title/artist to avoid adding duplicates already present on target."}
              </p>
            </div>
          </label>

          {duplicateTargetCount > 0 && skipDuplicates && (
            <p className="text-[11px] text-emerald-400/90 pl-6">
              {duplicateTargetCount} duplicate destination{" "}
              {duplicateTargetCount === 1 ? "match" : "matches"} in your selection will be skipped automatically.
            </p>
          )}

          {isYoutubeDuplicateConflict && (
            <div className="flex items-start gap-2 pl-6 text-[11px] text-amber-300">
              <AlertCircle className="w-3.5 h-3.5 shrink-0 mt-0.5 text-amber-400" />
              <span>
                YouTube Music does not support keeping duplicate track occurrences in a playlist. Re-enable Skip duplicate destination tracks or deduplicate the selection first.
              </span>
            </div>
          )}
        </div>

        {/* Breakdown Stats */}
        <div className="grid grid-cols-3 gap-2 text-center pt-1">
          <div className="p-2.5 rounded-lg bg-[#0c0c10] border border-white/[0.08]">
            <div className="text-base font-bold text-emerald-400 font-mono tabular-nums">
              {exactSelected}
            </div>
            <div className="text-[10px] text-zinc-500 uppercase mt-0.5">Exact Matches</div>
          </div>

          <div className="p-2.5 rounded-lg bg-[#0c0c10] border border-white/[0.08]">
            <div className="text-base font-bold text-amber-400 font-mono tabular-nums">
              {manualResolved}
            </div>
            <div className="text-[10px] text-zinc-500 uppercase mt-0.5">Resolved</div>
          </div>

          <div className="p-2.5 rounded-lg bg-[#0c0c10] border border-white/[0.08]">
            <div className="text-base font-bold text-zinc-400 font-mono tabular-nums">
              {skippedCount}
            </div>
            <div className="text-[10px] text-zinc-500 uppercase mt-0.5">Skipped</div>
          </div>
        </div>

        {/* Execution & Safety Notice */}
        <div className="p-3 rounded-lg bg-emerald-950/25 border border-emerald-500/20 text-emerald-300 text-xs space-y-2">
          <div className="flex items-start gap-2.5">
            <ShieldCheck className="w-4 h-4 text-emerald-400 shrink-0 mt-0.5" />
            <div className="text-[11px] leading-relaxed">
              <span className="font-semibold text-emerald-200">1-Click Snapshot Rollback: </span>
              SoundShift records a local snapshot and intent journal before remote writes, stopping for manual review if an outcome is uncertain.
            </div>
          </div>
          <div className="flex items-center justify-between pt-1 border-t border-emerald-500/15 text-[10px] text-zinc-400">
            <span>Track order guaranteed via sequential playlist writer</span>
            <Badge variant="neutral" className="font-mono text-[10px] py-0">
              {effectiveConcurrency} ordered worker
            </Badge>
          </div>
        </div>
      </div>
    </Modal>
  );
};
