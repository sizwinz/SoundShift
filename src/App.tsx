import { useState } from "react";
import { Titlebar } from "./components/layout/Titlebar";
import { Sidebar, NavTab } from "./components/layout/Sidebar";
import { AccountManager } from "./components/accounts/AccountManager";
import { AuthProvider, useAuth } from "./context/AuthContext";

function MainContent() {
  const [activeTab, setActiveTab] = useState<NavTab>("accounts");
  const { ytStatus, spotifyStatus } = useAuth();

  return (
    <div className="flex flex-col h-screen w-screen bg-[#000000] text-zinc-100 overflow-hidden font-sans">
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
        <main className="flex-1 overflow-y-auto bg-[#000000] p-6">
          <div className="max-w-5xl mx-auto">
            {activeTab === "accounts" && <AccountManager />}

            {activeTab === "playlists" && (
              <div className="border border-[#27272a] rounded-lg bg-[#09090b] p-6 text-zinc-300">
                <div className="text-sm font-medium mb-2">Playlists View</div>
                <p className="text-xs text-zinc-500 leading-relaxed">
                  Playlist selection and track extraction will be integrated in Phase 2. Connect accounts in the Accounts tab to prepare for library sync.
                </p>
              </div>
            )}

            {activeTab === "transfers" && (
              <div className="border border-[#27272a] rounded-lg bg-[#09090b] p-6 text-zinc-300">
                <div className="text-sm font-medium mb-2">Transfers & Diff Staging</div>
                <p className="text-xs text-zinc-500 leading-relaxed">
                  Deterministic track matching, diff inspection, and live worker pools will be integrated in Phase 3.
                </p>
              </div>
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
    </div>
  );
}

export default function App() {
  return (
    <AuthProvider>
      <MainContent />
    </AuthProvider>
  );
}
