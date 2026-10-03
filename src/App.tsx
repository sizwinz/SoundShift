import { useState } from "react";
import { Titlebar } from "./components/layout/Titlebar";
import { Sidebar, NavTab } from "./components/layout/Sidebar";
import { AccountManager } from "./components/accounts/AccountManager";
import { PlaylistSelector } from "./components/playlists/PlaylistSelector";
import { StagingTable } from "./components/diff/StagingTable";
import { TransferTargetOptions } from "./components/diff/TransferConfirmModal";
import { TelemetryDrawer } from "./components/transfer/TelemetryDrawer";
import { TransferHistory } from "./components/history/TransferHistory";
import { AuthProvider, useAuth } from "./context/AuthContext";
import { AudioProvider } from "./context/AudioContext";
import { TransferProvider, useTransfer } from "./context/TransferContext";
import { MatchResult } from "./types/diff";
import { Playlist } from "./types/provider";
import { ArrowRight, Music } from "lucide-react";

function MainContent() {
  const [activeTab, setActiveTab] = useState<NavTab>("accounts");
  const { ytStatus, spotifyStatus } = useAuth();
  const { startTransfer } = useTransfer();

  // Staged transfer data state
  const [stagedResults, setStagedResults] = useState<MatchResult[]>([]);
  const [activePlaylist, setActivePlaylist] = useState<Playlist | null>(null);
  const [sourceService, setSourceService] = useState<string>("spotify");
  const [targetService, setTargetService] = useState<string>("ytmusic");

  const handleMatchComplete = (
    results: MatchResult[],
    playlist: Playlist,
    source: string,
    target: string
  ) => {
    setStagedResults(results);
    setActivePlaylist(playlist);
    setSourceService(source);
    setTargetService(target);
    setActiveTab("transfers");
  };

  const handleConfirmTransfer = (
    selectedTracks: MatchResult[],
    options: TransferTargetOptions
  ) => {
    const tracksToTransfer = selectedTracks
      .map((r) => r.matched_track || r.source_track)
      .filter(Boolean);

    startTransfer({
      job_id: `job_${Date.now()}`,
      source_service: sourceService,
      target_service: targetService,
      source_playlist_name: activePlaylist?.title || "Playlist",
      target_playlist_name: options.playlistName,
      target_playlist_id: options.targetPlaylistId,
      is_new_playlist: options.isNewPlaylist,
      tracks: tracksToTransfer,
      concurrency: options.concurrency,
    });
  };

  return (
    <div className="flex flex-col h-screen w-screen bg-[#000000] text-zinc-100 overflow-hidden font-sans relative">
      {/* Custom Frameless Titlebar */}
      <Titlebar />

      {/* Main App Layout */}
      <div className="flex flex-1 overflow-hidden">
        {/* Responsive Collapsible Navigation Sidebar with live provider badges */}
        <Sidebar
          activeTab={activeTab}
          onTabChange={setActiveTab}
          ytStatus={ytStatus}
          spotifyStatus={spotifyStatus}
        />

        {/* Content Viewport */}
        <main className="flex-1 overflow-y-auto bg-[#000000] p-6 pb-20">
          <div className="max-w-5xl mx-auto">
            {activeTab === "accounts" && <AccountManager />}

            {activeTab === "playlists" && (
              <PlaylistSelector onMatchComplete={handleMatchComplete} />
            )}

            {activeTab === "transfers" && (
              stagedResults.length > 0 ? (
                <div className="flex flex-col gap-4">
                  <div className="flex items-center justify-between">
                    <div>
                      <h1 className="text-lg font-bold text-zinc-100">Review & Diff Staging</h1>
                      <p className="text-xs text-zinc-500">
                        Inspect matched tracks, resolve ambiguous candidates, and approve for migration.
                      </p>
                    </div>

                    <button
                      type="button"
                      onClick={() => setActiveTab("playlists")}
                      className="text-xs text-zinc-400 hover:text-zinc-200 transition-colors cursor-pointer flex items-center gap-1"
                    >
                      <span>Choose another playlist</span>
                      <ArrowRight className="w-3 h-3" />
                    </button>
                  </div>

                  <StagingTable
                    results={stagedResults}
                    onUpdateResults={setStagedResults}
                    onConfirmTransfer={handleConfirmTransfer}
                    playlistTitle={activePlaylist?.title}
                    sourceService={sourceService}
                    targetService={targetService}
                  />
                </div>
              ) : (
                <div className="border border-[#27272a] rounded-lg bg-[#09090b] p-8 text-center text-zinc-400">
                  <Music className="w-10 h-10 text-zinc-600 mx-auto mb-3" />
                  <div className="text-sm font-medium text-zinc-200 mb-1">
                    No Playlist Staged for Transfer
                  </div>
                  <p className="text-xs text-zinc-500 max-w-sm mx-auto mb-4">
                    Select a playlist and run the matching engine to review exact, ambiguous, and unmatched tracks before syncing.
                  </p>
                  <button
                    type="button"
                    onClick={() => setActiveTab("playlists")}
                    className="inline-flex items-center gap-2 px-4 py-2 bg-emerald-500 hover:bg-emerald-400 text-black text-xs font-semibold rounded cursor-pointer transition-colors"
                  >
                    <span>Browse Playlists</span>
                    <ArrowRight className="w-3.5 h-3.5" />
                  </button>
                </div>
              )
            )}

            {activeTab === "history" && (
              <TransferHistory onBrowsePlaylists={() => setActiveTab("playlists")} />
            )}

            {activeTab === "settings" && (
              <div className="border border-[#27272a] rounded-lg bg-[#09090b] p-6 text-zinc-300">
                <div className="text-sm font-medium mb-2">Application Settings</div>
                <p className="text-xs text-zinc-500 leading-relaxed">
                  Local-first configuration, concurrency settings, and snapshot rollback will be configured in Phase 4.
                </p>
              </div>
            )}
          </div>
        </main>
      </div>

      {/* Global Minimizable Telemetry Drawer pinned at bottom */}
      <TelemetryDrawer onNavigateToHistory={() => setActiveTab("history")} />
    </div>
  );
}

export default function App() {
  return (
    <AuthProvider>
      <AudioProvider>
        <TransferProvider>
          <MainContent />
        </TransferProvider>
      </AudioProvider>
    </AuthProvider>
  );
}
