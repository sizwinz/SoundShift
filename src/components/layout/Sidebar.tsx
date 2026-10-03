import { useState, useEffect } from "react";
import {
  ListMusic,
  ArrowLeftRight,
  History,
  ShieldCheck,
  Settings,
  ChevronLeft,
  ChevronRight,
  Radio,
} from "lucide-react";

export type NavTab = "playlists" | "transfers" | "history" | "accounts" | "settings";

interface SidebarProps {
  activeTab: NavTab;
  onTabChange: (tab: NavTab) => void;
  ytStatus?: "connected" | "expired" | "disconnected";
  spotifyStatus?: "connected" | "expired" | "disconnected";
}

export function Sidebar({
  activeTab,
  onTabChange,
  ytStatus = "disconnected",
  spotifyStatus = "disconnected",
}: SidebarProps) {
  // Default expanded on wide viewports (>= 1024px), collapsed on smaller
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
    { id: "accounts" as NavTab, label: "Accounts", icon: ShieldCheck },
    { id: "settings" as NavTab, label: "Settings", icon: Settings },
  ];

  const getStatusColor = (status: "connected" | "expired" | "disconnected") => {
    switch (status) {
      case "connected":
        return "bg-emerald-500";
      case "expired":
        return "bg-amber-500";
      case "disconnected":
      default:
        return "bg-zinc-600";
    }
  };

  return (
    <aside
      className={`h-[calc(100vh-2.25rem)] bg-[#09090b] border-r border-[#27272a] flex flex-col justify-between transition-all duration-200 select-none ${
        isCollapsed ? "w-16" : "w-[200px]"
      }`}
    >
      <div>
        {/* Toggle Collapse Button */}
        <div
          className={`flex items-center p-2 border-b border-[#27272a]/50 ${
            isCollapsed ? "justify-center" : "justify-end"
          }`}
        >
          <button
            type="button"
            onClick={() => setIsCollapsed(!isCollapsed)}
            className="p-1 text-zinc-400 hover:text-zinc-200 hover:bg-[#18181b] rounded transition-colors cursor-pointer"
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

        {/* Primary Navigation */}
        <nav className="p-2 space-y-1">
          {navItems.map((item) => {
            const Icon = item.icon;
            const isActive = activeTab === item.id;
            return (
              <button
                key={item.id}
                type="button"
                onClick={() => onTabChange(item.id)}
                className={`w-full flex items-center gap-3 px-3 py-2 rounded text-xs font-medium transition-colors ${
                  isActive
                    ? "bg-[#18181b] text-zinc-100 border border-[#27272a]"
                    : "text-zinc-400 hover:text-zinc-200 hover:bg-[#121215]"
                } ${isCollapsed ? "justify-center px-0" : ""}`}
                title={isCollapsed ? item.label : undefined}
              >
                <Icon className="w-4 h-4 shrink-0" />
                {!isCollapsed && <span>{item.label}</span>}
              </button>
            );
          })}
        </nav>
      </div>

      {/* Connection Status Section */}
      <div className="p-3 border-t border-[#27272a] bg-[#0c0c0e]">
        {!isCollapsed ? (
          <div className="space-y-2">
            <div className="text-[10px] font-mono uppercase tracking-wider text-zinc-500">
              Providers
            </div>
            <div className="flex items-center justify-between text-xs text-zinc-300">
              <span className="truncate">YouTube Music</span>
              <span
                className={`w-2 h-2 rounded-full ${getStatusColor(ytStatus)}`}
                title={`YouTube Music: ${ytStatus}`}
              />
            </div>
            <div className="flex items-center justify-between text-xs text-zinc-300">
              <span className="truncate">Spotify</span>
              <span
                className={`w-2 h-2 rounded-full ${getStatusColor(spotifyStatus)}`}
                title={`Spotify: ${spotifyStatus}`}
              />
            </div>
          </div>
        ) : (
          <div className="flex flex-col items-center gap-2">
            <Radio className="w-3.5 h-3.5 text-zinc-500" />
            <div className="flex flex-col gap-1.5 items-center">
              <span
                className={`w-2 h-2 rounded-full ${getStatusColor(ytStatus)}`}
                title={`YouTube Music: ${ytStatus}`}
              />
              <span
                className={`w-2 h-2 rounded-full ${getStatusColor(spotifyStatus)}`}
                title={`Spotify: ${spotifyStatus}`}
              />
            </div>
          </div>
        )}
      </div>
    </aside>
  );
}
