import React, { useState, useEffect, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { Playlist } from "../../types/provider";
import { MatchResult, MatchingProgressPayload } from "../../types/diff";
import { ServiceId } from "../../types/auth";
import { useAuth } from "../../context/AuthContext";
import {
  Music,
  ArrowRight,
  ArrowLeftRight,
  RefreshCw,
  Play,
  CheckCircle2,
  AlertCircle,
  Lock,
  Globe,
  Loader2,
} from "lucide-react";
import { ProviderLogo } from "../providers/ProviderLogo";
import { Button, Card, Badge, SearchInput } from "../ui";

interface PlaylistSelectorProps {
  onMatchComplete: (
    results: MatchResult[],
    playlist: Playlist,
    sourceService: string,
    targetService: string
  ) => void;
}

export const PlaylistSelector: React.FC<PlaylistSelectorProps> = ({
  onMatchComplete,
}) => {
  const { ytStatus, spotifyStatus, checkStatus } = useAuth();

  const [sourceService, setSourceService] = useState<string>("spotify");
  const [targetService, setTargetService] = useState<string>("ytmusic");

  const [playlists, setPlaylists] = useState<Playlist[]>([]);
  const [selectedPlaylist, setSelectedPlaylist] = useState<Playlist | null>(null);
  const [isLoadingPlaylists, setIsLoadingPlaylists] = useState(false);
  const [playlistError, setPlaylistError] = useState<string | null>(null);
  const [playlistSearch, setPlaylistSearch] = useState("");

  // Matching / Ingest Progress State
  const [isProcessing, setIsProcessing] = useState(false);
  const [progressStage, setProgressStage] = useState<"idle" | "ingesting" | "matching">("idle");
  const [progressMessage, setProgressMessage] = useState("");
  const [progressPercent, setProgressPercent] = useState(0);

  const isSourceConnected =
    (sourceService === "spotify" ? spotifyStatus : ytStatus) === "connected";
  const isTargetConnected =
    (targetService === "spotify" ? spotifyStatus : ytStatus) === "connected";

  const loadPlaylists = async (forceRefresh: boolean = false) => {
    if (!isSourceConnected) {
      setPlaylists([]);
      setSelectedPlaylist(null);
      return;
    }

    setIsLoadingPlaylists(true);
    setPlaylistError(null);

    try {
      const data = await invoke<Playlist[]>("list_provider_playlists", {
        service: sourceService,
        refresh: forceRefresh,
      });
      setPlaylists(data);
      if (data.length > 0 && (!selectedPlaylist || !data.some((p) => p.id === selectedPlaylist.id))) {
        setSelectedPlaylist(data[0]);
      } else if (data.length === 0) {
        setSelectedPlaylist(null);
      }
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      setPlaylistError(message);
      if (message.toLowerCase().includes("no active session")) {
        checkStatus(sourceService as ServiceId);
      }
    } finally {
      setIsLoadingPlaylists(false);
    }
  };

  useEffect(() => {
    loadPlaylists(false);
  }, [sourceService, isSourceConnected]);

  const handleSourceChange = (newSource: string) => {
    if (newSource === sourceService) return;
    setSourceService(newSource);
    setTargetService(newSource === "spotify" ? "ytmusic" : "spotify");
    setSelectedPlaylist(null);
    setPlaylists([]);
    setPlaylistError(null);
  };

  const handleSwapServices = () => {
    const prevSource = sourceService;
    setSourceService(targetService);
    setTargetService(prevSource);
    setSelectedPlaylist(null);
    setPlaylists([]);
    setPlaylistError(null);
  };

  const handleStartMatching = async () => {
    if (!selectedPlaylist || !isSourceConnected || !isTargetConnected) return;

    setIsProcessing(true);
    setProgressStage("ingesting");
    setProgressMessage(`Ingesting ${selectedPlaylist.title} tracks...`);
    setProgressPercent(10);
    setPlaylistError(null);

    let unlistenIngest: UnlistenFn | null = null;
    let unlistenMatching: UnlistenFn | null = null;

    try {
      unlistenIngest = await listen<{
        playlist_id: string;
        loaded: number;
        tracks: unknown[];
      }>("playlist:ingest_progress", (event) => {
        const loaded = event.payload.loaded;
        const total = selectedPlaylist.track_count || 1;
        const pct = Math.min(45, Math.round((loaded / Math.max(total, 1)) * 45));
        setProgressPercent(pct);
        setProgressMessage(`Ingested ${loaded} of ${total} tracks...`);
      });

      await invoke("fetch_playlist_tracks", {
        service: sourceService,
        playlistId: selectedPlaylist.id,
        refresh: false,
      });

      setProgressStage("matching");
      setProgressMessage("Analyzing matches across target catalog...");
      setProgressPercent(50);

      unlistenMatching = await listen<MatchingProgressPayload>(
        "matching:progress",
        (event) => {
          const { processed, total, current_track, current_result } = event.payload;
          const pct = 50 + Math.round((processed / Math.max(total, 1)) * 48);
          setProgressPercent(pct);
          const trackTitle = current_track || current_result?.source_track?.title;
          if (trackTitle) {
            setProgressMessage(`Matching "${trackTitle}" (${processed} of ${total})...`);
          } else {
            setProgressMessage(`Matching track ${processed} of ${total}...`);
          }
        }
      );

      const results = await invoke<MatchResult[]>("execute_playlist_matching", {
        sourceService,
        targetService,
        playlistId: selectedPlaylist.id,
      });

      setProgressPercent(100);
      setProgressMessage("Matching complete. Loading staging diff...");

      setTimeout(() => {
        onMatchComplete(results, selectedPlaylist, sourceService, targetService);
      }, 300);
    } catch (err: unknown) {
      setPlaylistError(err instanceof Error ? err.message : String(err));
      setIsProcessing(false);
      setProgressStage("idle");
    } finally {
      if (unlistenIngest) unlistenIngest();
      if (unlistenMatching) unlistenMatching();
    }
  };

  const filteredPlaylists = useMemo(() => {
    if (!playlistSearch.trim()) return playlists;
    const query = playlistSearch.toLowerCase();
    return playlists.filter((p) => p.title.toLowerCase().includes(query));
  }, [playlists, playlistSearch]);

  return (
    <div className="flex flex-col gap-5 max-w-[1440px] mx-auto w-full pb-20">
      {/* Route Selector Card */}
      <Card className="p-4 bg-[#0a0a0e] border-white/[0.08]">
        <div className="flex flex-col md:flex-row items-stretch md:items-center justify-between gap-4">
          {/* Source Selection */}
          <div className="flex-1 flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <span className="text-[10px] font-mono uppercase tracking-wider text-zinc-500 font-semibold">
                Source Catalog
              </span>
              <Badge
                variant={isSourceConnected ? "connected" : "disconnected"}
                icon={isSourceConnected ? <CheckCircle2 className="w-3 h-3 text-emerald-400" /> : <AlertCircle className="w-3 h-3 text-zinc-500" />}
              >
                {isSourceConnected ? "Connected" : "Disconnected"}
              </Badge>
            </div>

            <div className="grid grid-cols-2 gap-2">
              <button
                type="button"
                onClick={() => handleSourceChange("spotify")}
                disabled={isProcessing}
                className={`flex items-center justify-center gap-2 p-2.5 rounded-lg border text-xs font-medium transition-all cursor-pointer ${
                  sourceService === "spotify"
                    ? "bg-[#181822] text-zinc-100 border-white/[0.2] shadow-sm ring-1 ring-white/[0.1]"
                    : "bg-[#0f0f14] text-zinc-400 border-white/[0.06] hover:bg-[#14141c]"
                }`}
              >
                <ProviderLogo provider="spotify" size="sm" />
                <span>Spotify</span>
              </button>

              <button
                type="button"
                onClick={() => handleSourceChange("ytmusic")}
                disabled={isProcessing}
                className={`flex items-center justify-center gap-2 p-2.5 rounded-lg border text-xs font-medium transition-all cursor-pointer ${
                  sourceService === "ytmusic"
                    ? "bg-[#181822] text-zinc-100 border-white/[0.2] shadow-sm ring-1 ring-white/[0.1]"
                    : "bg-[#0f0f14] text-zinc-400 border-white/[0.06] hover:bg-[#14141c]"
                }`}
              >
                <ProviderLogo provider="ytmusic" size="sm" />
                <span>YouTube Music</span>
              </button>
            </div>
          </div>

          {/* Swap Direction Button */}
          <div className="flex items-center justify-center pt-2 md:pt-4">
            <button
              type="button"
              onClick={handleSwapServices}
              disabled={isProcessing}
              className="p-2 rounded-lg bg-white/[0.04] hover:bg-white/[0.08] border border-white/[0.08] text-zinc-400 hover:text-zinc-100 transition-colors cursor-pointer"
              title="Swap transfer direction"
              aria-label="Swap transfer direction"
            >
              <ArrowLeftRight className="w-4 h-4" />
            </button>
          </div>

          {/* Destination Target */}
          <div className="flex-1 flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <span className="text-[10px] font-mono uppercase tracking-wider text-zinc-500 font-semibold">
                Destination Target
              </span>
              <Badge
                variant={isTargetConnected ? "connected" : "disconnected"}
                icon={isTargetConnected ? <CheckCircle2 className="w-3 h-3 text-emerald-400" /> : <AlertCircle className="w-3 h-3 text-zinc-500" />}
              >
                {isTargetConnected ? "Connected" : "Disconnected"}
              </Badge>
            </div>

            <div className="flex items-center justify-between p-2.5 rounded-lg border bg-[#0f0f14] border-white/[0.06] text-xs h-[42px]">
              <div className="flex items-center gap-2.5 font-medium text-zinc-200 capitalize">
                <ProviderLogo
                  provider={targetService === "spotify" ? "spotify" : "ytmusic"}
                  size="sm"
                />
                <span>{targetService === "spotify" ? "Spotify" : "YouTube Music"}</span>
              </div>
              <span className="text-[11px] text-zinc-500 font-mono">Target Catalog</span>
            </div>
          </div>
        </div>
      </Card>

      {/* Error Alert */}
      {playlistError && (
        <div className="p-3.5 bg-rose-950/40 border border-rose-500/40 rounded-xl text-rose-300 text-xs flex items-start gap-3">
          <AlertCircle className="w-4 h-4 shrink-0 mt-0.5" />
          <div className="flex-1">
            <div className="font-semibold mb-0.5">Operation Notice</div>
            <div className="text-rose-400/90 leading-relaxed text-[11px]">{playlistError}</div>
          </div>
        </div>
      )}

      {/* Ingestion & Matching Progress Banner */}
      {isProcessing && (
        <Card elevated className="p-4 border-emerald-500/30 bg-[#0e1210]">
          <div className="flex items-center justify-between mb-2">
            <div className="flex items-center gap-2">
              <Loader2 className="w-4 h-4 text-emerald-400 animate-spin" />
              <span className="text-xs font-semibold text-zinc-100">
                {progressStage === "ingesting" ? "Extracting Track Metadata" : "Executing Deterministic Matching"}
              </span>
            </div>
            <span className="text-xs font-mono text-emerald-400 font-semibold tabular-nums">{progressPercent}%</span>
          </div>

          <p className="text-xs text-zinc-400 mb-3">{progressMessage}</p>

          <div className="w-full bg-black/60 h-2 rounded-full overflow-hidden border border-white/[0.06]">
            <div
              className="bg-emerald-500 h-full transition-all duration-300 ease-out"
              style={{ width: `${progressPercent}%` }}
            />
          </div>
        </Card>
      )}

      {/* Playlist Grid Header with Search */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 pt-1">
        <div>
          <h3 className="text-sm font-semibold text-zinc-100">Select Playlist to Migrate</h3>
          <p className="text-xs text-zinc-500">
            Choose a playlist from {sourceService === "spotify" ? "Spotify" : "YouTube Music"} to stage for transfer.
          </p>
        </div>

        <div className="flex items-center gap-2">
          {isSourceConnected && (
            <SearchInput
              value={playlistSearch}
              onChange={(e) => setPlaylistSearch(e.target.value)}
              onClear={() => setPlaylistSearch("")}
              placeholder="Search playlists..."
              className="w-48 sm:w-56"
            />
          )}

          <Button
            variant="secondary"
            size="sm"
            onClick={() => loadPlaylists(true)}
            disabled={isLoadingPlaylists || isProcessing || !isSourceConnected}
          >
            <RefreshCw className={`w-3.5 h-3.5 ${isLoadingPlaylists ? "animate-spin" : ""}`} />
            <span className="hidden sm:inline">Refresh</span>
          </Button>
        </div>
      </div>

      {/* Not Connected Warning */}
      {!isSourceConnected && (
        <Card className="p-10 border-dashed border-white/[0.1] text-center bg-[#09090c]/40">
          <Music className="w-10 h-10 text-zinc-600 mx-auto mb-3" />
          <p className="text-xs text-zinc-300 font-medium mb-1">
            {sourceService === "spotify" ? "Spotify" : "YouTube Music"} account is not connected.
          </p>
          <p className="text-[11px] text-zinc-500 max-w-sm mx-auto">
            Connect your streaming account using the Accounts button in the sidebar to load your library playlists.
          </p>
        </Card>
      )}

      {/* Playlists Grid */}
      {isSourceConnected && (
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
          {isLoadingPlaylists && playlists.length === 0 ? (
            <div className="col-span-full py-16 flex flex-col items-center justify-center text-zinc-500">
              <Loader2 className="w-7 h-7 animate-spin mb-3 text-emerald-400" />
              <p className="text-xs text-zinc-400">Loading playlists from {sourceService}...</p>
            </div>
          ) : filteredPlaylists.length === 0 ? (
            <div className="col-span-full py-16 text-center text-zinc-500">
              <p className="text-xs">No playlists found matching your search.</p>
            </div>
          ) : (
            filteredPlaylists.map((playlist) => {
              const isSelected = selectedPlaylist?.id === playlist.id;

              return (
                <div
                  key={playlist.id}
                  onClick={() => !isProcessing && setSelectedPlaylist(playlist)}
                  className={`group relative flex items-center gap-3 p-3 rounded-xl border transition-all cursor-pointer ${
                    isSelected
                      ? "bg-[#14141c] border-emerald-500/50 shadow-md ring-1 ring-emerald-500/30"
                      : "bg-[#0b0b0f] border-white/[0.08] hover:border-white/[0.16] hover:bg-[#101016]"
                  }`}
                >
                  {/* Playlist Cover Art */}
                  {playlist.cover_url ? (
                    <img
                      src={playlist.cover_url}
                      alt=""
                      className="w-12 h-12 rounded-lg object-cover shrink-0 bg-zinc-900 border border-white/[0.06]"
                      loading="lazy"
                    />
                  ) : (
                    <div className="w-12 h-12 rounded-lg bg-zinc-900 border border-white/[0.08] shrink-0 flex items-center justify-center text-zinc-500">
                      <Music className="w-5 h-5" />
                    </div>
                  )}

                  {/* Playlist Details */}
                  <div className="min-w-0 flex-1">
                    <div className="text-xs font-semibold text-zinc-100 truncate group-hover:text-emerald-300 transition-colors">
                      {playlist.title}
                    </div>
                    <div className="text-[11px] text-zinc-500 flex items-center gap-1.5 mt-0.5 font-mono">
                      <span className="tabular-nums">{playlist.track_count} tracks</span>
                      <span>•</span>
                      <span className="flex items-center gap-1 font-sans">
                        {playlist.is_public ? (
                          <>
                            <Globe className="w-3 h-3 text-zinc-500" /> Public
                          </>
                        ) : (
                          <>
                            <Lock className="w-3 h-3 text-zinc-500" /> Private
                          </>
                        )}
                      </span>
                    </div>
                  </div>

                  {/* Radio check indicator */}
                  <div
                    className={`w-4 h-4 rounded-full border flex items-center justify-center shrink-0 ${
                      isSelected
                        ? "border-emerald-500 bg-emerald-500 text-black"
                        : "border-white/[0.2] bg-black/40"
                    }`}
                  >
                    {isSelected && <div className="w-1.5 h-1.5 rounded-full bg-black" />}
                  </div>
                </div>
              );
            })
          )}
        </div>
      )}

      {/* Floating Action Footer when Playlist is Selected */}
      {isSourceConnected && (
        <div className="fixed inset-x-0 bottom-0 z-40 h-16 border-t border-white/[0.08] bg-[#08080a]/95 px-4 shadow-[0_-12px_36px_rgba(0,0,0,0.7)] backdrop-blur-xl lg:left-[210px]">
          <div className="mx-auto flex h-full max-w-[1440px] items-center justify-between gap-4">
            <div className="min-w-0 text-xs text-zinc-400">
              {selectedPlaylist ? (
                <span className="block truncate">
                  Ready to analyze: <strong className="text-zinc-100 font-semibold">{selectedPlaylist.title}</strong>
                  <span className="text-zinc-500 font-mono ml-1.5 tabular-nums">({selectedPlaylist.track_count} tracks)</span>
                </span>
              ) : (
                <span>Select a playlist to proceed</span>
              )}
            </div>

            <Button
              variant="primary"
              size="md"
              onClick={handleStartMatching}
              disabled={!selectedPlaylist || isProcessing || !isTargetConnected}
              className="shrink-0"
            >
              <Play className="w-3.5 h-3.5 fill-current" />
              <span className="hidden sm:inline">Analyze & Match Playlist</span>
              <span className="sm:hidden">Analyze</span>
              <ArrowRight className="w-3.5 h-3.5 ml-1" />
            </Button>
          </div>
        </div>
      )}
    </div>
  );
};
