import { useState } from "react";
import { Titlebar } from "./components/layout/Titlebar";
import { Sidebar, NavTab } from "./components/layout/Sidebar";
import { AccountManager } from "./components/accounts/AccountManager";
import { PlaylistSelector } from "./components/playlists/PlaylistSelector";
import { StagingTable } from "./components/diff/StagingTable";
import { TransferTargetOptions } from "./components/diff/TransferConfirmModal";
import { TelemetryDrawer } from "./components/transfer/TelemetryDrawer";
import { TransferHistory } from "./components/history/TransferHistory";
import { SettingsPage } from "./components/settings/SettingsPage";
import { AuthProvider, useAuth } from "./context/AuthContext";
import { AudioProvider } from "./context/AudioContext";
import { TransferProvider, useTransfer } from "./context/TransferContext";
import { MatchResult } from "./types/diff";
import { Playlist } from "./types/provider";
import { ArrowRight, Music } from "lucide-react";
import { Modal, Button, Card } from "./components/ui";

function MainContent() {
  const [activeTab, setActiveTab] = useState<NavTab>("playlists");
  const [isAccountsModalOpen, setIsAccountsModalOpen] = useState(false);
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
      .filter((r) => r.status === "Exact" && r.matched_track)
      .map((r) => r.matched_track!)
      .filter((track) => track.is_playable !== false);

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
      skip_duplicates: options.skipDuplicates ?? true,
    });
  };

  const getBreadcrumb = () => {
    switch (activeTab) {
      case "playlists":
        return "Playlists";
      case "transfers":
        return activePlaylist ? `Review & Diff / ${activePlaylist.title}` : "Review & Diff Staging";
      case "history":
        return "Transfer History & Snapshots";
      case "accounts":
        return "Streaming Accounts";
      case "settings":
        return "Settings & Preferences";
      default:
        return undefined;
    }
  };

  return (
    <div className="flex flex-col min-h-[100dvh] h-screen w-screen bg-[#000000] text-zinc-100 overflow-hidden font-sans relative">
      {/* Custom Frameless Titlebar with Breadcrumb */}
      <Titlebar breadcrumb={getBreadcrumb()} />

      {/* Main App Layout */}
      <div className="flex flex-1 overflow-hidden">
        {/* Responsive Collapsible Navigation Sidebar with live provider footer */}
        <Sidebar
          activeTab={activeTab}
          onTabChange={setActiveTab}
          onOpenAccounts={() => setIsAccountsModalOpen(true)}
          ytStatus={ytStatus}
          spotifyStatus={spotifyStatus}
        />

        {/* Content Viewport & Telemetry Column */}
        <div className="flex-1 flex flex-col min-w-0 min-h-0 h-full overflow-hidden relative">
          {activeTab === "transfers" && stagedResults.length > 0 ? (
            <main className="flex-1 min-w-0 min-h-0 overflow-hidden flex flex-col bg-[#000000]">
              <StagingTable
                results={stagedResults}
                onUpdateResults={setStagedResults}
                onConfirmTransfer={handleConfirmTransfer}
                playlistTitle={activePlaylist?.title}
                sourceService={sourceService}
                targetService={targetService}
                onBackToPlaylists={() => setActiveTab("playlists")}
              />
            </main>
          ) : (
            <main className="flex-1 min-w-0 min-h-0 overflow-y-auto bg-[#000000] scrollbar-thin scrollbar-thumb-zinc-800 scrollbar-track-transparent">
              <div className="w-full max-w-[1440px] mx-auto p-4 sm:p-5 lg:p-6">
                {activeTab === "accounts" && <AccountManager />}

                {activeTab === "playlists" && (
                  <PlaylistSelector onMatchComplete={handleMatchComplete} />
                )}

                {activeTab === "transfers" && (
                  <div className="min-h-[60vh] flex items-center justify-center">
                    <Card className="p-12 text-center text-zinc-400 bg-[#09090d]/40 border-dashed border-white/[0.1] max-w-xl mx-auto">
                      <Music className="w-10 h-10 text-zinc-600 mx-auto mb-3" />
                      <div className="text-sm font-semibold text-zinc-200 mb-1">
                        No Playlist Staged for Transfer
                      </div>
                      <p className="text-xs text-zinc-500 max-w-sm mx-auto mb-5 leading-relaxed">
                        Select a playlist and run the matching engine to review exact, ambiguous, and unmatched tracks before syncing.
                      </p>
                      <Button
                        variant="primary"
                        size="sm"
                        onClick={() => setActiveTab("playlists")}
                      >
                        <span>Browse Playlists</span>
                        <ArrowRight className="w-3.5 h-3.5" />
                      </Button>
                    </Card>
                  </div>
                )}

                {activeTab === "history" && (
                  <TransferHistory onBrowsePlaylists={() => setActiveTab("playlists")} />
                )}

                {activeTab === "settings" && <SettingsPage />}
              </div>
            </main>
          )}

          {/* Global Docked Telemetry & Activity Bar */}
          <TelemetryDrawer onNavigateToHistory={() => setActiveTab("history")} />
        </div>
      </div>

      {/* Quick-Access Accounts Modal (accessible anywhere via sidebar) */}
      <Modal
        isOpen={isAccountsModalOpen}
        onClose={() => setIsAccountsModalOpen(false)}
        title="Streaming Accounts"
        description="Connect or manage your Spotify and YouTube Music sessions."
        maxWidth="max-w-2xl"
      >
        <AccountManager isModal />
      </Modal>
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
