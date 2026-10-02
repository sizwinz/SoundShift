import { useState } from "react";
import { Titlebar } from "./components/layout/Titlebar";
import { Sidebar, NavTab } from "./components/layout/Sidebar";

export default function App() {
  const [activeTab, setActiveTab] = useState<NavTab>("accounts");

  return (
    <div className="flex flex-col h-screen w-screen bg-[#000000] text-zinc-100 overflow-hidden font-sans">
      {/* Custom Frameless Titlebar */}
      <Titlebar />

      {/* Main App Layout */}
      <div className="flex flex-1 overflow-hidden">
        {/* Responsive Collapsible Navigation Sidebar */}
        <Sidebar activeTab={activeTab} onTabChange={setActiveTab} />

        {/* Content Viewport */}
        <main className="flex-1 overflow-y-auto bg-[#000000] p-6">
          <div className="max-w-5xl mx-auto">
            <header className="mb-6">
              <h1 className="text-xl font-bold tracking-tight text-zinc-100">
                {activeTab === "playlists" && "Playlists"}
                {activeTab === "transfers" && "Transfers & History"}
                {activeTab === "accounts" && "Connected Accounts"}
                {activeTab === "settings" && "Application Settings"}
              </h1>
              <p className="text-xs text-zinc-400 mt-1">
                SoundShift Local-first Music Migration System
              </p>
            </header>

            {/* Placeholder Content for Scaffold */}
            <div className="border border-[#27272a] rounded-lg bg-[#09090b] p-6 text-zinc-300">
              <div className="text-sm font-medium mb-2">Desktop Shell Initialized</div>
              <p className="text-xs text-zinc-500 leading-relaxed">
                Tauri v2 runtime with React 19, TypeScript, embedded SQLite WAL database, and AMOLED design system. Ready for native WebView authentication trap integration.
              </p>
            </div>
          </div>
        </main>
      </div>
    </div>
  );
}
