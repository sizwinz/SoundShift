import React from "react";
import { Search, CheckSquare, Square } from "lucide-react";

export type FilterTab = "all" | "exact" | "amber" | "red" | "duplicates";

export interface FilterCounts {
  all: number;
  exact: number;
  amber: number;
  red: number;
  duplicates: number;
}

interface FilterToolbarProps {
  activeTab: FilterTab;
  onTabChange: (tab: FilterTab) => void;
  searchQuery: string;
  onSearchChange: (query: string) => void;
  counts: FilterCounts;
  isAllFilteredSelected: boolean;
  onToggleSelectAllFiltered: () => void;
}

export const FilterToolbar: React.FC<FilterToolbarProps> = ({
  activeTab,
  onTabChange,
  searchQuery,
  onSearchChange,
  counts,
  isAllFilteredSelected,
  onToggleSelectAllFiltered,
}) => {
  const tabs: { id: FilterTab; label: string; count: number; colorClass?: string }[] = [
    { id: "all", label: "All Tracks", count: counts.all },
    {
      id: "exact",
      label: "Exact",
      count: counts.exact,
      colorClass: "text-emerald-400 bg-emerald-950/60 border-emerald-500/30",
    },
    {
      id: "amber",
      label: "Needs Review",
      count: counts.amber,
      colorClass: "text-amber-400 bg-amber-950/60 border-amber-500/30",
    },
    {
      id: "red",
      label: "Unmatched",
      count: counts.red,
      colorClass: "text-rose-400 bg-rose-950/60 border-rose-500/30",
    },
    {
      id: "duplicates",
      label: "Duplicates",
      count: counts.duplicates,
      colorClass: "text-zinc-400 bg-zinc-800 border-zinc-700",
    },
  ];

  return (
    <div className="flex flex-col sm:flex-row items-stretch sm:items-center justify-between gap-3 px-4 py-3 bg-[#09090b] border-b border-[#27272a]">
      {/* Filter Tabs */}
      <div className="flex items-center gap-1.5 overflow-x-auto scrollbar-none py-0.5">
        <button
          type="button"
          onClick={onToggleSelectAllFiltered}
          className="flex items-center gap-1.5 px-2.5 py-1 rounded text-xs text-zinc-400 hover:text-zinc-200 hover:bg-[#18181b] border border-[#27272a] mr-2 transition-colors cursor-pointer flex-shrink-0"
          title={isAllFilteredSelected ? "Deselect all visible" : "Select all visible"}
        >
          {isAllFilteredSelected ? (
            <CheckSquare className="w-3.5 h-3.5 text-emerald-400" />
          ) : (
            <Square className="w-3.5 h-3.5 text-zinc-500" />
          )}
          <span className="hidden md:inline">Select Visible</span>
        </button>

        {tabs.map((tab) => {
          const isActive = activeTab === tab.id;
          return (
            <button
              key={tab.id}
              type="button"
              onClick={() => onTabChange(tab.id)}
              className={`flex items-center gap-1.5 px-3 py-1 rounded text-xs font-medium transition-colors cursor-pointer flex-shrink-0 border ${
                isActive
                  ? "bg-zinc-800 text-zinc-100 border-zinc-600 shadow-xs"
                  : "bg-transparent text-zinc-400 hover:text-zinc-200 border-transparent hover:bg-zinc-900"
              }`}
            >
              <span>{tab.label}</span>
              <span
                className={`px-1.5 py-0.2 rounded-full text-[10px] font-mono border ${
                  tab.colorClass ?? "bg-zinc-800 text-zinc-400 border-zinc-700"
                }`}
              >
                {tab.count}
              </span>
            </button>
          );
        })}
      </div>

      {/* Search Input */}
      <div className="relative flex-shrink-0 w-full sm:w-64">
        <Search className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-zinc-500 pointer-events-none" />
        <input
          type="text"
          value={searchQuery}
          onChange={(e) => onSearchChange(e.target.value)}
          placeholder="Filter tracks by title, artist..."
          className="w-full bg-[#121215] border border-[#27272a] rounded pl-8 pr-3 py-1 text-xs text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-zinc-500 transition-colors"
        />
      </div>
    </div>
  );
};
