import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { Playlist } from "../../types/provider";
import { MatchResult, MatchingProgressPayload } from "../../types/diff";
import { useAuth } from "../../context/AuthContext";
import {
  Music,
  ArrowRight,
  RefreshCw,
  Play,
  CheckCircle2,
  AlertCircle,
  Lock,
  Globe,
  Loader2,
} from "lucide-react";

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
  const { ytStatus, spotifyStatus } = useAuth();

  const [sourceService, setSourceService] = useState<string>("spotify");
  const [targetService, setTargetService] = useState<string>("ytmusic");

  const [playlists, setPlaylists] = useState<Playlist[]>([]);
  const [selectedPlaylist, setSelectedPlaylist] = useState<Playlist | null>(null);
  const [isLoadingPlaylists, setIsLoadingPlaylists] = useState(false);
  const [playlistError, setPlaylistError] = useState<string | null>(null);

  // Matching / Ingest Progress State
  const [isProcessing, setIsProcessing] = useState(false);
  const [progressStage, setProgressStage] = useState<"idle" | "ingesting" | "matching">("idle");
  const [progressMessage, setProgressMessage] = useState("");
  const [progressPercent, setProgressPercent] = useState(0);

  // Check which services are authenticated
  const isSourceConnected =
    (sourceService === "spotify" ? spotifyStatus : ytStatus) === "connected";
  const isTargetConnected =
    (targetService === "spotify" ? spotifyStatus : ytStatus) === "connected";

  // Load playlists for source service
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
      }
    } catch (err: unknown) {
      setPlaylistError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsLoadingPlaylists(false);
    }
  };

  useEffect(() => {
    loadPlaylists(false);
  }, [sourceService, isSourceConnected]);

  // Handle service switch
  const handleSourceChange = (newSource: string) => {
    setSourceService(newSource);
    setTargetService(newSource === "spotify" ? "ytmusic" : "spotify");
  };

  // Run ingestion and deterministic matching
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
      // Listen to streaming ingestion chunks
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

      // Pass 1: Ingest tracks
      await invoke("fetch_playlist_tracks", {
        service: sourceService,
        playlistId: selectedPlaylist.id,
        refresh: false,
      });

      // Pass 2: Matching
      setProgressStage("matching");
      setProgressMessage("Analyzing matches across target catalog...");
      setProgressPercent(50);

      // Listen to streaming matching progress
      unlistenMatching = await listen<MatchingProgressPayload>(
        "matching:progress",
        (event) => {
          const { processed, total } = event.payload;
          const pct = 50 + Math.round((processed / Math.max(total, 1)) * 48);
          setProgressPercent(pct);
          setProgressMessage(`Matching track ${processed} of ${total}...`);
        }
      );

      const results = await invoke<MatchResult[]>("execute_playlist_matching", {
        sourceService,
        targetService,
        playlistId: selectedPlaylist.id,
      });

      setProgressPercent(100);
      setProgressMessage("Matching complete. Loading staging diff...");

      // Small delay for smooth UI transition
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

  return (
    <div className="flex flex-col gap-6">
      {/* Route & Direction Bar */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        {/* Source Provider Card */}
        <div className="bg-[#09090b] border border-[#27272a] rounded-lg p-4">
          <div className="flex items-center justify-between mb-3">
            <span className="text-xs font-semibold uppercase tracking-wider text-zinc-400">
              Source Service
            </span>
            <span
              className={`inline-flex items-center gap-1 text-[11px] font-medium px-2 py-0.5 rounded-full border ${
                isSourceConnected
                  ? "bg-emerald-950/60 text-emerald-400 border-emerald-500/30"
                  : "bg-rose-950/60 text-rose-400 border-rose-500/30"
              }`}
            >
              {isSourceConnected ? (
                <>
                  <CheckCircle2 className="w-3 h-3" /> Connected
                </>
              ) : (
                <>
                  <AlertCircle className="w-3 h-3" /> Disconnected
                </>
              )}
            </span>
          </div>

          <div className="grid grid-cols-2 gap-2">
            <button
              type="button"
              onClick={() => handleSourceChange("spotify")}
              disabled={isProcessing}
              className={`flex items-center justify-center gap-2 p-2.5 rounded border text-xs font-medium transition-colors cursor-pointer ${
                sourceService === "spotify"
                  ? "bg-zinc-800 text-zinc-100 border-zinc-500 shadow-xs"
                  : "bg-zinc-900/50 text-zinc-400 border-zinc-800 hover:bg-zinc-800/60"
              }`}
            >
              <span className="w-2 h-2 rounded-full bg-[#1DB954]" />
              Spotify
            </button>

            <button
              type="button"
              onClick={() => handleSourceChange("ytmusic")}
              disabled={isProcessing}
              className={`flex items-center justify-center gap-2 p-2.5 rounded border text-xs font-medium transition-colors cursor-pointer ${
                sourceService === "ytmusic"
                  ? "bg-zinc-800 text-zinc-100 border-zinc-500 shadow-xs"
                  : "bg-zinc-900/50 text-zinc-400 border-zinc-800 hover:bg-zinc-800/60"
              }`}
            >
              <span className="w-2 h-2 rounded-full bg-[#FF0000]" />
              YouTube Music
            </button>
          </div>
        </div>

        {/* Destination Provider Card */}
        <div className="bg-[#09090b] border border-[#27272a] rounded-lg p-4">
          <div className="flex items-center justify-between mb-3">
            <span className="text-xs font-semibold uppercase tracking-wider text-zinc-400">
              Destination Target
            </span>
            <span
              className={`inline-flex items-center gap-1 text-[11px] font-medium px-2 py-0.5 rounded-full border ${
                isTargetConnected
                  ? "bg-emerald-950/60 text-emerald-400 border-emerald-500/30"
                  : "bg-rose-950/60 text-rose-400 border-rose-500/30"
              }`}
            >
              {isTargetConnected ? (
                <>
                  <CheckCircle2 className="w-3 h-3" /> Connected
                </>
              ) : (
                <>
                  <AlertCircle className="w-3 h-3" /> Disconnected
                </>
              )}
            </span>
          </div>

          <div className="flex items-center justify-between p-2.5 rounded border bg-zinc-900/50 border-zinc-800 text-xs">
            <div className="flex items-center gap-2 font-medium text-zinc-200 capitalize">
              <span
                className={`w-2 h-2 rounded-full ${
                  targetService === "spotify" ? "bg-[#1DB954]" : "bg-[#FF0000]"
                }`}
              />
              {targetService === "spotify" ? "Spotify" : "YouTube Music"}
            </div>
            <span className="text-[11px] text-zinc-500">Destination target</span>
          </div>
        </div>
      </div>

      {/* Error Banner */}
      {playlistError && (
        <div className="p-3.5 bg-rose-950/60 border border-rose-500/40 rounded-lg text-rose-300 text-xs flex items-start gap-2.5">
          <AlertCircle className="w-4 h-4 flex-shrink-0 mt-0.5" />
          <div className="flex-1">
            <div className="font-medium mb-0.5">Execution Failed</div>
            <div className="text-rose-400/90 leading-relaxed">{playlistError}</div>
          </div>
        </div>
      )}

      {/* Active Processing Modal / Progress Banner */}
      {isProcessing && (
        <div className="bg-[#09090b] border border-emerald-500/40 rounded-lg p-5 shadow-2xl">
          <div className="flex items-center justify-between mb-2">
            <div className="flex items-center gap-2">
              <Loader2 className="w-4 h-4 text-emerald-400 animate-spin" />
              <span className="text-xs font-semibold text-zinc-200">
                {progressStage === "ingesting" ? "Extracting Track Metadata" : "Executing Matching Engine"}
              </span>
            </div>
            <span className="text-xs font-mono text-emerald-400">{progressPercent}%</span>
          </div>

          <p className="text-xs text-zinc-400 mb-3">{progressMessage}</p>

          <div className="w-full bg-zinc-900 h-2 rounded-full overflow-hidden border border-zinc-800">
            <div
              className="bg-emerald-500 h-full transition-all duration-300 ease-out"
              style={{ width: `${progressPercent}%` }}
            />
          </div>
        </div>
      )}

      {/* Playlist Grid Header */}
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-sm font-semibold text-zinc-100">Select Playlist to Migrate</h3>
          <p className="text-xs text-zinc-500">
            Choose a source playlist to inspect metadata and stage deterministic matches.
          </p>
        </div>

        <button
          type="button"
          onClick={() => loadPlaylists(true)}
          disabled={isLoadingPlaylists || isProcessing || !isSourceConnected}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-zinc-900 hover:bg-zinc-800 border border-[#27272a] text-xs text-zinc-300 transition-colors cursor-pointer disabled:opacity-50"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${isLoadingPlaylists ? "animate-spin" : ""}`} />
          <span>Refresh List</span>
        </button>
      </div>

      {/* Not Connected Warning */}
      {!isSourceConnected && (
        <div className="p-8 border border-dashed border-[#27272a] rounded-lg text-center bg-[#09090b]/50">
          <Music className="w-8 h-8 text-zinc-600 mx-auto mb-2" />
          <p className="text-xs text-zinc-400 mb-1">
            Source account ({sourceService === "spotify" ? "Spotify" : "YouTube Music"}) is not connected.
          </p>
          <p className="text-[11px] text-zinc-500">
            Go to the Accounts tab in the sidebar to authenticate your session.
          </p>
        </div>
      )}

      {/* Playlists Grid */}
      {isSourceConnected && (
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
          {isLoadingPlaylists && playlists.length === 0 ? (
            <div className="col-span-full py-12 flex flex-col items-center justify-center text-zinc-500">
              <Loader2 className="w-6 h-6 animate-spin mb-2" />
              <p className="text-xs">Loading playlists from {sourceService}...</p>
            </div>
          ) : playlists.length === 0 ? (
            <div className="col-span-full py-12 text-center text-zinc-500">
              <p className="text-xs">No playlists found for this account.</p>
            </div>
          ) : (
            playlists.map((playlist) => {
              const isSelected = selectedPlaylist?.id === playlist.id;

              return (
                <div
                  key={playlist.id}
                  onClick={() => !isProcessing && setSelectedPlaylist(playlist)}
                  className={`group relative flex items-center gap-3 p-3 rounded-lg border transition-all cursor-pointer ${
                    isSelected
                      ? "bg-[#121215] border-emerald-500/50 shadow-md ring-1 ring-emerald-500/30"
                      : "bg-[#09090b] border-[#27272a] hover:border-zinc-700 hover:bg-[#0e0e11]"
                  }`}
                >
                  {/* Playlist Cover Art */}
                  {playlist.cover_url ? (
                    <img
                      src={playlist.cover_url}
                      alt=""
                      className="w-12 h-12 rounded object-cover flex-shrink-0 bg-zinc-900 border border-zinc-800"
                      loading="lazy"
                    />
                  ) : (
                    <div className="w-12 h-12 rounded bg-zinc-800 border border-zinc-700 flex-shrink-0 flex items-center justify-center text-zinc-500">
                      <Music className="w-5 h-5" />
                    </div>
                  )}

                  {/* Playlist Details */}
                  <div className="min-w-0 flex-1">
                    <div className="text-xs font-semibold text-zinc-200 truncate group-hover:text-zinc-100">
                      {playlist.title}
                    </div>
                    <div className="text-[11px] text-zinc-500 flex items-center gap-1.5 mt-0.5">
                      <span>{playlist.track_count} tracks</span>
                      <span>•</span>
                      <span className="flex items-center gap-0.5">
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
                    className={`w-4 h-4 rounded-full border flex items-center justify-center flex-shrink-0 ${
                      isSelected
                        ? "border-emerald-500 bg-emerald-500 text-black"
                        : "border-zinc-700 bg-zinc-900"
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

      {/* Action Footer */}
      {isSourceConnected && (
        <div className="flex items-center justify-between p-4 bg-[#09090b] border border-[#27272a] rounded-lg mt-2">
          <div className="text-xs text-zinc-400">
            {selectedPlaylist ? (
              <span>
                Selected: <strong className="text-zinc-200">{selectedPlaylist.title}</strong> (
                {selectedPlaylist.track_count} tracks)
              </span>
            ) : (
              <span>Select a playlist to proceed</span>
            )}
          </div>

          <button
            type="button"
            onClick={handleStartMatching}
            disabled={!selectedPlaylist || isProcessing || !isTargetConnected}
            className="flex items-center gap-2 px-5 py-2 rounded bg-emerald-500 hover:bg-emerald-400 text-black font-semibold text-xs transition-colors cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed shadow-md"
          >
            <Play className="w-3.5 h-3.5 fill-black" />
            <span>Analyze & Match Playlist</span>
            <ArrowRight className="w-3.5 h-3.5" />
          </button>
        </div>
      )}
    </div>
  );
};
