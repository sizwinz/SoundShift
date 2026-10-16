import React from "react";
import { CheckSquare, Square } from "lucide-react";
import { Badge, SearchInput, BadgeVariant } from "../ui";

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
  const tabs: { id: FilterTab; label: string; count: number; badgeVariant: BadgeVariant }[] = [
    { id: "all", label: "All Tracks", count: counts.all, badgeVariant: "neutral" },
    { id: "exact", label: "Exact", count: counts.exact, badgeVariant: "exact" },
    { id: "amber", label: "Needs Review", count: counts.amber, badgeVariant: "amber" },
    { id: "red", label: "Unmatched", count: counts.red, badgeVariant: "unmatched" },
    { id: "duplicates", label: "Duplicates", count: counts.duplicates, badgeVariant: "duplicate" },
  ];

  return (
    <div className="flex flex-col sm:flex-row items-stretch sm:items-center justify-between gap-3 px-4 py-2.5 bg-[#09090d] border-b border-white/[0.08]">
      {/* Filter Tabs */}
      <div className="flex items-center gap-1.5 overflow-x-auto scrollbar-none py-0.5">
        <button
          type="button"
          onClick={onToggleSelectAllFiltered}
          className="flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs text-zinc-400 hover:text-zinc-100 hover:bg-white/[0.06] border border-white/[0.08] mr-1.5 transition-colors cursor-pointer shrink-0"
          title={isAllFilteredSelected ? "Deselect all visible" : "Select all visible"}
        >
          {isAllFilteredSelected ? (
            <CheckSquare className="w-3.5 h-3.5 text-emerald-400" />
          ) : (
            <Square className="w-3.5 h-3.5 text-zinc-500" />
          )}
          <span className="hidden md:inline font-medium">Select Visible</span>
        </button>

        {tabs.map((tab) => {
          const isActive = activeTab === tab.id;
          return (
            <button
              key={tab.id}
              type="button"
              onClick={() => onTabChange(tab.id)}
              className={`flex items-center gap-2 px-3 py-1 rounded-lg text-xs font-medium transition-all cursor-pointer shrink-0 border ${
                isActive
                  ? "bg-[#161620] text-zinc-100 border-white/[0.14] shadow-xs"
                  : "bg-transparent text-zinc-400 hover:text-zinc-200 border-transparent hover:bg-white/[0.04]"
              }`}
            >
              <span>{tab.label}</span>
              <Badge variant={tab.badgeVariant} className="px-1.5 py-0 text-[10px]">
                {tab.count}
              </Badge>
            </button>
          );
        })}
      </div>

      {/* Search Input */}
      <SearchInput
        value={searchQuery}
        onChange={(e) => onSearchChange(e.target.value)}
        onClear={() => onSearchChange("")}
        placeholder="Filter tracks by title, artist..."
        className="w-full sm:w-64 shrink-0"
      />
    </div>
  );
};
