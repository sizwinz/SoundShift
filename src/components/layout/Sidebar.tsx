import { useState, useEffect } from "react";
import {
  ListMusic,
  ArrowLeftRight,
  History,
  Settings,
  ChevronLeft,
  ChevronRight,
  Radio,
  ExternalLink,
} from "lucide-react";
import { ProviderLogo } from "../providers/ProviderLogo";

export type NavTab = "playlists" | "transfers" | "history" | "accounts" | "settings";

interface SidebarProps {
  activeTab: NavTab;
  onTabChange: (tab: NavTab) => void;
  onOpenAccounts?: () => void;
  ytStatus?: "connected" | "expired" | "disconnected";
  spotifyStatus?: "connected" | "expired" | "disconnected";
}

export function Sidebar({
  activeTab,
  onTabChange,
  onOpenAccounts,
  ytStatus = "disconnected",
  spotifyStatus = "disconnected",
}: SidebarProps) {
  const [isCollapsed, setIsCollapsed] = useState(false);

  useEffect(() => {
    const handleResize = () => {
      if (window.innerWidth < 1024) {
        setIsCollapsed(true);
      } else {
        setIsCollapsed(false);
      }
    };

    handleResize();
    window.addEventListener("resize", handleResize);
    return () => window.removeEventListener("resize", handleResize);
  }, []);

  const navItems = [
    { id: "playlists" as NavTab, label: "Playlists", icon: ListMusic },
    { id: "transfers" as NavTab, label: "Transfers", icon: ArrowLeftRight },
    { id: "history" as NavTab, label: "History", icon: History },
    { id: "settings" as NavTab, label: "Settings", icon: Settings },
  ];

  const getStatusIndicator = (status: "connected" | "expired" | "disconnected") => {
    if (status === "connected") {
      return <span className="w-2 h-2 rounded-full bg-emerald-400 shadow-[0_0_8px_rgba(16,185,129,0.5)]" />;
    }
    if (status === "expired") {
      return <span className="w-2 h-2 rounded-full bg-amber-400 animate-pulse" />;
    }
    return <span className="w-2 h-2 rounded-full bg-zinc-600" />;
  };

  return (
    <aside
      className={`h-full shrink-0 bg-[#08080a] border-r border-white/[0.08] flex flex-col justify-between transition-all duration-200 select-none ${
        isCollapsed ? "w-16" : "w-[210px]"
      }`}
    >
      <div>
        {/* Collapse Toggle */}
        <div
          className={`flex items-center px-3 py-2.5 border-b border-white/[0.06] ${
            isCollapsed ? "justify-center" : "justify-between"
          }`}
        >
          {!isCollapsed && (
            <span className="text-[10px] font-mono uppercase tracking-wider text-zinc-500 font-semibold">
              Navigation
            </span>
          )}
          <button
            type="button"
            onClick={() => setIsCollapsed(!isCollapsed)}
            className="p-1 text-zinc-400 hover:text-zinc-100 hover:bg-white/[0.06] rounded-md transition-colors cursor-pointer"
            title={isCollapsed ? "Expand Sidebar" : "Collapse Sidebar"}
            aria-label={isCollapsed ? "Expand Sidebar" : "Collapse Sidebar"}
          >
            {isCollapsed ? (
              <ChevronRight className="w-4 h-4" />
            ) : (
              <ChevronLeft className="w-4 h-4" />
            )}
          </button>
        </div>

        {/* Primary Nav List */}
        <nav className="p-2 space-y-1">
          {navItems.map((item) => {
            const Icon = item.icon;
            const isActive = activeTab === item.id;
            return (
              <button
                key={item.id}
                type="button"
                onClick={() => onTabChange(item.id)}
                className={`w-full flex items-center gap-3 px-3 py-2 rounded-lg text-xs font-medium transition-all duration-150 cursor-pointer ${
                  isActive
                    ? "bg-[#14141a] text-zinc-100 border border-white/[0.1] shadow-xs"
                    : "text-zinc-400 hover:text-zinc-200 hover:bg-white/[0.04] border border-transparent"
                } ${isCollapsed ? "justify-center px-0" : ""}`}
                title={isCollapsed ? item.label : undefined}
              >
                <Icon className={`w-4 h-4 shrink-0 ${isActive ? "text-emerald-400" : ""}`} />
                {!isCollapsed && <span>{item.label}</span>}
              </button>
            );
          })}
        </nav>
      </div>

      {/* Connection Status Section */}
      <div className="p-2.5 border-t border-white/[0.08] bg-[#0c0c10]/90">
        {!isCollapsed ? (
          <div className="space-y-2">
            <div className="flex items-center justify-between px-1">
              <span className="text-[10px] font-mono uppercase tracking-wider text-zinc-500 font-semibold">
                Accounts
              </span>
              <button
                type="button"
                onClick={() => (onOpenAccounts ? onOpenAccounts() : onTabChange("accounts"))}
                className="text-[10px] text-zinc-500 hover:text-zinc-300 flex items-center gap-1 cursor-pointer transition-colors"
                title="Manage accounts"
              >
                <span>Manage</span>
                <ExternalLink className="w-2.5 h-2.5" />
              </button>
            </div>

            {/* Provider Cards */}
            <div className="space-y-1.5">
              <div
                onClick={() => (onOpenAccounts ? onOpenAccounts() : onTabChange("accounts"))}
                className="flex items-center justify-between p-2 rounded-lg bg-[#111116] border border-white/[0.06] hover:border-white/[0.12] transition-colors cursor-pointer text-xs"
              >
                <div className="flex items-center gap-2 truncate">
                  <ProviderLogo provider="ytmusic" size="sm" />
                  <span className="truncate text-zinc-300 font-medium text-[11px]">YouTube Music</span>
                </div>
                {getStatusIndicator(ytStatus)}
              </div>

              <div
                onClick={() => (onOpenAccounts ? onOpenAccounts() : onTabChange("accounts"))}
                className="flex items-center justify-between p-2 rounded-lg bg-[#111116] border border-white/[0.06] hover:border-white/[0.12] transition-colors cursor-pointer text-xs"
              >
                <div className="flex items-center gap-2 truncate">
                  <ProviderLogo provider="spotify" size="sm" />
                  <span className="truncate text-zinc-300 font-medium text-[11px]">Spotify</span>
                </div>
                {getStatusIndicator(spotifyStatus)}
              </div>
            </div>
          </div>
        ) : (
          <div className="flex flex-col items-center gap-2.5 py-1">
            <Radio className="w-3.5 h-3.5 text-zinc-500" />
            <button
              type="button"
              onClick={() => (onOpenAccounts ? onOpenAccounts() : onTabChange("accounts"))}
              className="flex flex-col gap-2 items-center cursor-pointer p-1 rounded hover:bg-white/[0.06]"
              title="Manage Accounts"
            >
              <div className="relative">
                <ProviderLogo provider="ytmusic" size="sm" />
                <span className="absolute -bottom-0.5 -right-0.5">
                  {getStatusIndicator(ytStatus)}
                </span>
              </div>
              <div className="relative">
                <ProviderLogo provider="spotify" size="sm" />
                <span className="absolute -bottom-0.5 -right-0.5">
                  {getStatusIndicator(spotifyStatus)}
                </span>
              </div>
            </button>
          </div>
        )}
      </div>
    </aside>
  );
}
