import { useState, useEffect, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Titlebar } from "./components/layout/Titlebar";
import { Sidebar, NavTab } from "./components/layout/Sidebar";
import { AccountManager } from "./components/accounts/AccountManager";
import { PlaylistSelector } from "./components/playlists/PlaylistSelector";
import { StagingTable } from "./components/diff/StagingTable";
import { TransferTargetOptions } from "./components/diff/TransferConfirmModal";
import { TelemetryDrawer } from "./components/transfer/TelemetryDrawer";
import { TransferHistory } from "./components/history/TransferHistory";
import { SettingsPage, readSoundShiftSettings } from "./components/settings/SettingsPage";
import { SyncPage } from "./components/sync/SyncPage";
import { AuthProvider, useAuth } from "./context/AuthContext";
import { AudioProvider } from "./context/AudioContext";
import { TransferProvider, useTransfer } from "./context/TransferContext";
import { MatchResult } from "./types/diff";
import { Playlist } from "./types/provider";
import { isTauri } from "./utils/tauri";
import { ArrowRight, Music } from "lucide-react";
import { Modal, Button, Card } from "./components/ui";
import { CommandPalette, PaletteCommand } from "./components/ui/CommandPalette";

const STAGED_SESSION_KEY = "soundshift.staged-session";

interface PersistedStagedSession {
  results: MatchResult[];
  playlist: Playlist | null;
  sourceService: string;
  targetService: string;
}

function readStagedSession(): PersistedStagedSession | null {
  try {
    const raw = sessionStorage.getItem(STAGED_SESSION_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as PersistedStagedSession;
    if (!Array.isArray(parsed.results) || parsed.results.length === 0) return null;
    return parsed;
  } catch {
    return null;
  }
}

function MainContent() {
  const initialSession = readStagedSession();
  const [activeTab, setActiveTab] = useState<NavTab>("playlists");
  const [isAccountsModalOpen, setIsAccountsModalOpen] = useState(false);
  const [isCommandPaletteOpen, setIsCommandPaletteOpen] = useState(false);
  const { ytStatus, spotifyStatus } = useAuth();
  const { stage, progress, startTransfer } = useTransfer();

  const [stagedResults, setStagedResults] = useState<MatchResult[]>(
    () => initialSession?.results ?? []
  );
  const [activePlaylist, setActivePlaylist] = useState<Playlist | null>(
    () => initialSession?.playlist ?? null
  );
  const [sourceService, setSourceService] = useState<string>(
    () => initialSession?.sourceService ?? "spotify"
  );
  const [targetService, setTargetService] = useState<string>(
    () => initialSession?.targetService ?? "ytmusic"
  );

  useEffect(() => {
    const settings = readSoundShiftSettings();
    document.documentElement.classList.toggle("reduce-motion", settings.reducedMotion);
    if (isTauri()) {
      void invoke("set_tray_preference", { enabled: settings.minimizeToTray }).catch(() => {});
    }
  }, []);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setIsCommandPaletteOpen((open) => !open);
      } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        window.dispatchEvent(new CustomEvent("soundshift:focus-search"));
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  useEffect(() => {
    try {
      if (stagedResults.length === 0) {
        sessionStorage.removeItem(STAGED_SESSION_KEY);
        return;
      }
      sessionStorage.setItem(
        STAGED_SESSION_KEY,
        JSON.stringify({
          results: stagedResults,
          playlist: activePlaylist,
          sourceService,
          targetService,
        })
      );
    } catch {
      // Ignore storage quota errors cleanly
    }
  }, [stagedResults, activePlaylist, sourceService, targetService]);

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
      case "sync":
        return "Playlist Sync";
      case "accounts":
        return "Streaming Accounts";
      case "settings":
        return "Settings & Preferences";
      default:
        return undefined;
    }
  };

  const unresolvedStagedCount = stagedResults.filter((r) => r.status !== "Exact").length;

  const activeTitlebarStatus =
    stage === "transferring" || stage === "starting"
      ? {
          label: progress
            ? `Transferring ${progress.processed}/${progress.total}`
            : "Transferring...",
          tone: "emerald" as const,
          onClick: () => setActiveTab("transfers"),
        }
      : stage === "paused" || stage === "pausing"
      ? {
          label: "Transfer paused",
          tone: "amber" as const,
          onClick: () => setActiveTab("transfers"),
        }
      : stage === "recovery_required" || stage === "failed"
      ? {
          label: "Recovery required",
          tone: "rose" as const,
          onClick: () => setActiveTab("history"),
        }
      : null;

  const paletteCommands: PaletteCommand[] = useMemo(
    () => [
      {
        id: "nav-playlists",
        label: "Go to Playlists",
        hint: "Select source playlist, paste URL, or import backup file",
        keywords: "browse import url csv json m3u8",
        onSelect: () => setActiveTab("playlists"),
      },
      {
        id: "nav-transfers",
        label: "Go to Review & Diff Staging",
        hint:
          stagedResults.length > 0
            ? `${stagedResults.length} tracks staged (${unresolvedStagedCount} unresolved)`
            : "Inspect matched tracks before transfer",
        keywords: "diff review staging tracks",
        onSelect: () => setActiveTab("transfers"),
      },
      {
        id: "nav-sync",
        label: "Go to Playlist Sync",
        hint: "Link two playlists and manage hourly two-way sync",
        keywords: "two way link schedule",
        onSelect: () => setActiveTab("sync"),
      },
      {
        id: "nav-history",
        label: "Go to Transfer History & Snapshots",
        hint: "Inspect recovery journals, audit integrity, and trigger rollback",
        keywords: "rollback audit recovery logs",
        onSelect: () => setActiveTab("history"),
      },
      {
        id: "nav-settings",
        label: "Go to Settings & Preferences",
        hint: "Matching workers, confirmation prompt, reduced motion, system tray",
        keywords: "preferences tray concurrency cache",
        onSelect: () => setActiveTab("settings"),
      },
      {
        id: "open-accounts",
        label: "Open Streaming Accounts",
        hint: "Connect or reconnect Spotify and YouTube Music sessions",
        keywords: "auth login spotify youtube",
        onSelect: () => setIsAccountsModalOpen(true),
      },
      {
        id: "import-file",
        label: "Import Playlist Backup (.json / .csv / .m3u8)",
        hint: "Open local playlist file to match against target catalog",
        keywords: "restore backup csv m3u json",
        onSelect: () => {
          setActiveTab("playlists");
          window.setTimeout(() => {
            window.dispatchEvent(new CustomEvent("soundshift:open-import"));
          }, 0);
        },
      },
      {
        id: "focus-search",
        label: "Focus Search Bar (Ctrl+F)",
        hint: "Filter current view by playlist or track",
        keywords: "find filter query",
        onSelect: () => {
          window.dispatchEvent(new CustomEvent("soundshift:focus-search"));
        },
      },
    ],
    [stagedResults.length, unresolvedStagedCount]
  );

  return (
    <>
      <div id="app-shell" className="flex flex-col min-h-[100dvh] h-screen w-screen bg-[#000000] text-zinc-100 overflow-hidden font-sans relative">
        {/* Custom Frameless Titlebar with Breadcrumb */}
        <Titlebar
          breadcrumb={getBreadcrumb()}
          onBreadcrumbClick={activeTab === "transfers" ? () => setActiveTab("playlists") : undefined}
          activeStatus={activeTitlebarStatus}
        />

        {/* Main App Layout */}
        <div className="flex flex-1 overflow-hidden">
          <Sidebar
            activeTab={activeTab}
            onTabChange={setActiveTab}
            onOpenAccounts={() => setIsAccountsModalOpen(true)}
            ytStatus={ytStatus}
            spotifyStatus={spotifyStatus}
            badges={{
              transfers: unresolvedStagedCount > 0 ? unresolvedStagedCount : undefined,
              history: stage === "recovery_required" ? 1 : undefined,
            }}
          />

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
            ) : activeTab === "playlists" ? (
              <main className="flex-1 min-w-0 min-h-0 overflow-hidden flex flex-col bg-[#000000]">
                <PlaylistSelector onMatchComplete={handleMatchComplete} />
              </main>
            ) : (
              <main className="flex-1 min-w-0 min-h-0 overflow-y-auto bg-[#000000] scrollbar-thin scrollbar-thumb-zinc-800 scrollbar-track-transparent">
                <div className="w-full max-w-[1440px] mx-auto p-4 sm:p-5 lg:p-6">
                  {activeTab === "accounts" && <AccountManager />}

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
                  {activeTab === "sync" && <SyncPage />}
                </div>
              </main>
            )}

            <TelemetryDrawer onNavigateToHistory={() => setActiveTab("history")} />
          </div>
        </div>
      </div>

      {/* Quick-Access Accounts Modal */}
      <Modal
        isOpen={isAccountsModalOpen}
        onClose={() => setIsAccountsModalOpen(false)}
        title="Streaming Accounts"
        description="Connect or manage your Spotify and YouTube Music sessions."
        maxWidth="max-w-2xl"
      >
        <AccountManager isModal />
      </Modal>

      {/* Global Ctrl+K Command Palette */}
      <CommandPalette
        isOpen={isCommandPaletteOpen}
        onClose={() => setIsCommandPaletteOpen(false)}
        commands={paletteCommands}
      />
    </>
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
