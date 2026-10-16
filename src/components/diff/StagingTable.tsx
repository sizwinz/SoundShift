import React, { useRef, useMemo, useState, useEffect } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { MatchResult } from "../../types/diff";
import { SourceTrack } from "../../types/provider";
import { DiffRow } from "./DiffRow";
import { FilterToolbar, FilterTab, FilterCounts } from "./FilterToolbar";
import { DisambiguationDrawer } from "./DisambiguationDrawer";
import { StagingActionBar } from "./StagingActionBar";
import { TransferConfirmModal, TransferTargetOptions } from "./TransferConfirmModal";
import { Music, CheckCircle2, ArrowRight, ArrowLeft } from "lucide-react";
import { Badge } from "../ui";

interface StagingTableProps {
  results: MatchResult[];
  onUpdateResults?: (newResults: MatchResult[]) => void;
  onOpenDrawer?: (result: MatchResult) => void;
  onConfirmTransfer?: (selectedTracks: MatchResult[], options: TransferTargetOptions) => void;
  selectedTrackIds?: Set<string>;
  onSelectionChange?: (selectedIds: Set<string>) => void;
  playlistTitle?: string;
  sourceService?: string;
  targetService?: string;
  onBackToPlaylists?: () => void;
}

export const StagingTable: React.FC<StagingTableProps> = ({
  results,
  onUpdateResults,
  onOpenDrawer,
  onConfirmTransfer,
  selectedTrackIds: externalSelectedIds,
  onSelectionChange,
  playlistTitle,
  sourceService,
  targetService,
  onBackToPlaylists,
}) => {
  const parentRef = useRef<HTMLDivElement>(null);

  const [activeTab, setActiveTab] = useState<FilterTab>("all");
  const [searchQuery, setSearchQuery] = useState("");
  const [activeDrawerTrack, setActiveDrawerTrack] = useState<MatchResult | null>(null);

  const handleOpenDrawer = (result: MatchResult) => {
    setActiveDrawerTrack(result);
    onOpenDrawer?.(result);
  };

  const handleResolveCandidate = (sourceTrackId: string, resolvedTrack: SourceTrack) => {
    const updatedResults = results.map((r) => {
      if (r.source_track.id === sourceTrackId) {
        return {
          ...r,
          status: "Exact" as const,
          matched_track: resolvedTrack,
          confidence: 1.0,
          match_method: "manual_disambiguation",
        };
      }
      return r;
    });

    const nextSelection = new Set(selectedTrackIds);
    nextSelection.add(sourceTrackId);
    updateSelection(nextSelection);

    if (onUpdateResults) {
      onUpdateResults(updatedResults);
    }
    setActiveDrawerTrack(null);
  };

  const [internalSelectedIds, setInternalSelectedIds] = useState<Set<string>>(() => {
    const initial = new Set<string>();
    results.forEach((r) => {
      if (r.status === "Exact") {
        initial.add(r.source_track.id);
      }
    });
    return initial;
  });

  const selectedTrackIds = externalSelectedIds ?? internalSelectedIds;

  const updateSelection = (newSet: Set<string>) => {
    if (onSelectionChange) {
      onSelectionChange(newSet);
    } else {
      setInternalSelectedIds(newSet);
    }
  };

  const [isConfirmModalOpen, setIsConfirmModalOpen] = useState(false);

  const handleAcceptAllAmbiguous = () => {
    const nextSelection = new Set(selectedTrackIds);
    const updated = results.map((r) => {
      if (
        r.status === "Ambiguous" &&
        r.candidates.length > 0 &&
        r.candidates[0].similarity >= 0.85 &&
        r.candidates[0].duration_delta_ms <= 4_000
      ) {
        const top = r.candidates[0];
        nextSelection.add(r.source_track.id);
        return {
          ...r,
          status: "Exact" as const,
          matched_track: top.track,
          confidence: top.similarity,
          match_method: "batch_accepted",
        };
      }
      return r;
    });

    updateSelection(nextSelection);
    if (onUpdateResults) {
      onUpdateResults(updated);
    }
  };

  const handleSkipUnresolved = () => {
    const nextSelection = new Set<string>();
    results.forEach((r) => {
      if (r.status === "Exact" && selectedTrackIds.has(r.source_track.id)) {
        nextSelection.add(r.source_track.id);
      }
    });
    updateSelection(nextSelection);
  };

  const handleDeduplicate = () => {
    const nextSelection = new Set(selectedTrackIds);
    duplicateIdSet.forEach((id) => {
      nextSelection.delete(id);
    });
    updateSelection(nextSelection);
  };

  useEffect(() => {
    if (!externalSelectedIds) {
      const initial = new Set<string>();
      results.forEach((r) => {
        if (r.status === "Exact") {
          initial.add(r.source_track.id);
        }
      });
      setInternalSelectedIds(initial);
    }
  }, [results, externalSelectedIds]);

  const duplicateIdSet = useMemo(() => {
    const seen = new Set<string>();
    const duplicates = new Set<string>();

    results.forEach((r) => {
      const key = r.source_track.isrc
        ? `isrc:${r.source_track.isrc.trim().toUpperCase()}`
        : `title:${r.source_track.title.toLowerCase().trim()}::${(r.source_track.artists[0] || "").toLowerCase().trim()}`;

      if (seen.has(key)) {
        duplicates.add(r.source_track.id);
      } else {
        seen.add(key);
      }
    });

    return duplicates;
  }, [results]);

  const counts: FilterCounts = useMemo(() => {
    let exact = 0;
    let amber = 0;
    let red = 0;
    let duplicates = 0;

    results.forEach((r) => {
      if (r.status === "Exact") exact++;
      else if (r.status === "Ambiguous") amber++;
      else if (r.status === "NotFound") red++;

      if (duplicateIdSet.has(r.source_track.id)) {
        duplicates++;
      }
    });

    return {
      all: results.length,
      exact,
      amber,
      red,
      duplicates,
    };
  }, [results, duplicateIdSet]);

  const filteredResults = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();

    return results.filter((r) => {
      if (activeTab === "exact" && r.status !== "Exact") return false;
      if (activeTab === "amber" && r.status !== "Ambiguous") return false;
      if (activeTab === "red" && r.status !== "NotFound") return false;
      if (activeTab === "duplicates" && !duplicateIdSet.has(r.source_track.id)) return false;

      if (query) {
        const titleMatch = r.source_track.title.toLowerCase().includes(query);
        const artistMatch = r.source_track.artists.some((a) =>
          a.toLowerCase().includes(query)
        );
        const destTitleMatch = r.matched_track?.title.toLowerCase().includes(query);
        const destArtistMatch = r.matched_track?.artists.some((a) =>
          a.toLowerCase().includes(query)
        );
        if (!titleMatch && !artistMatch && !destTitleMatch && !destArtistMatch) {
          return false;
        }
      }

      return true;
    });
  }, [results, activeTab, searchQuery, duplicateIdSet]);

  const rowVirtualizer = useVirtualizer({
    count: filteredResults.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 76,
    overscan: 10,
  });

  const handleToggleSelect = (trackId: string) => {
    const next = new Set(selectedTrackIds);
    if (next.has(trackId)) {
      next.delete(trackId);
    } else {
      next.add(trackId);
    }
    updateSelection(next);
  };

  const isAllFilteredSelected = useMemo(() => {
    if (filteredResults.length === 0) return false;
    return filteredResults.every((r) => selectedTrackIds.has(r.source_track.id));
  }, [filteredResults, selectedTrackIds]);

  const handleToggleSelectAllFiltered = () => {
    const next = new Set(selectedTrackIds);
    if (isAllFilteredSelected) {
      filteredResults.forEach((r) => next.delete(r.source_track.id));
    } else {
      filteredResults.forEach((r) => next.add(r.source_track.id));
    }
    updateSelection(next);
  };

  return (
    <div className="flex flex-col flex-1 min-h-0 h-full w-full overflow-hidden bg-[#000000]">
      {/* Table Header Bar */}
      <div className="w-full bg-[#0a0a0e] border-b border-white/[0.08] shrink-0">
        <div className="w-full max-w-[1440px] mx-auto px-4 sm:px-6 py-2.5 flex items-center justify-between">
          <div className="flex items-center gap-3 min-w-0">
            {onBackToPlaylists && (
              <button
                type="button"
                onClick={onBackToPlaylists}
                className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg text-xs font-medium text-zinc-400 hover:text-zinc-100 bg-white/[0.04] hover:bg-white/[0.08] border border-white/[0.06] transition-colors cursor-pointer shrink-0"
                title="Return to Playlists"
              >
                <ArrowLeft className="w-3.5 h-3.5" />
                <span className="hidden sm:inline">Playlists</span>
              </button>
            )}
            <div className="w-8 h-8 rounded-lg bg-white/[0.04] border border-white/[0.08] flex items-center justify-center text-zinc-400 shrink-0">
              <Music className="w-3.5 h-3.5 text-emerald-400" />
            </div>
            <div className="min-w-0">
              <h2 className="text-sm font-semibold text-zinc-100 flex items-center gap-2 truncate">
                <span className="truncate">{playlistTitle || "Playlist Staging Diff"}</span>
                <span className="text-xs font-mono font-normal text-zinc-500 tabular-nums shrink-0">
                  ({results.length} total tracks)
                </span>
              </h2>
              <div className="text-[11px] text-zinc-500 flex items-center gap-1.5 mt-0.5">
                <span>From</span>
                <span className="capitalize text-zinc-300 font-medium">{sourceService || "Unknown"}</span>
                <ArrowRight className="w-2.5 h-2.5 text-zinc-600" />
                <span>To</span>
                <span className="capitalize text-zinc-300 font-medium">{targetService || "Unknown"}</span>
              </div>
            </div>
          </div>

          <div className="flex items-center gap-2 shrink-0">
            <Badge variant="connected" icon={<CheckCircle2 className="w-3.5 h-3.5 text-emerald-400" />}>
              <span>{selectedTrackIds.size} selected</span>
            </Badge>
          </div>
        </div>
      </div>

      {/* Filter Toolbar */}
      <div className="w-full bg-[#09090d] border-b border-white/[0.08] shrink-0">
        <div className="w-full max-w-[1440px] mx-auto">
          <FilterToolbar
            activeTab={activeTab}
            onTabChange={setActiveTab}
            searchQuery={searchQuery}
            onSearchChange={setSearchQuery}
            counts={counts}
            isAllFilteredSelected={isAllFilteredSelected}
            onToggleSelectAllFiltered={handleToggleSelectAllFiltered}
          />
        </div>
      </div>

      {/* Column Headers */}
      <div className="w-full bg-[#08080c] border-b border-white/[0.04] shrink-0">
        <div className="w-full max-w-[1440px] mx-auto px-4 sm:px-6 h-8 flex items-center text-[10px] font-mono font-semibold text-zinc-500 uppercase tracking-wider select-none">
          <div className="w-7 mr-3"></div>
          <div className="flex-1">Source Track</div>
          <div className="w-36 text-center">Match Status</div>
          <div className="flex-1 text-right">Destination Track</div>
        </div>
      </div>

      {/* Virtualized List Container - Full Width so scrollbar is directly against right window edge */}
      <div
        ref={parentRef}
        className="relative min-h-0 flex-1 w-full overflow-y-auto scrollbar-thin scrollbar-thumb-zinc-800 scrollbar-track-transparent bg-[#07070a]"
      >
        {filteredResults.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-64 text-center px-4">
            <div className="w-10 h-10 rounded-full bg-white/[0.04] border border-white/[0.08] flex items-center justify-center mb-3 text-zinc-500">
              <Music className="w-5 h-5 opacity-40" />
            </div>
            {searchQuery ? (
              <>
                <p className="text-xs font-medium text-zinc-300 mb-1">
                  No tracks matching &ldquo;{searchQuery}&rdquo;
                </p>
                <p className="text-[11px] text-zinc-500 mb-3 max-w-xs">
                  Try checking for typos or searching by artist name instead.
                </p>
                <button
                  type="button"
                  onClick={() => setSearchQuery("")}
                  className="px-3 py-1 rounded-md text-xs font-medium bg-white/[0.06] hover:bg-white/[0.1] text-zinc-300 transition-colors cursor-pointer"
                >
                  Clear search
                </button>
              </>
            ) : activeTab === "amber" ? (
              <>
                <p className="text-xs font-medium text-zinc-300 mb-1">
                  No Ambiguous Matches Requiring Review
                </p>
                <p className="text-[11px] text-zinc-500 mb-3 max-w-xs">
                  All tracks in this playlist were either matched with high confidence or have no candidates.
                </p>
                <button
                  type="button"
                  onClick={() => setActiveTab("all")}
                  className="px-3 py-1 rounded-md text-xs font-medium bg-white/[0.06] hover:bg-white/[0.1] text-zinc-300 transition-colors cursor-pointer"
                >
                  View All Tracks ({results.length})
                </button>
              </>
            ) : activeTab === "red" ? (
              <>
                <p className="text-xs font-medium text-zinc-300 mb-1">
                  No Unmatched Tracks
                </p>
                <p className="text-[11px] text-zinc-500 mb-3 max-w-xs">
                  Every track in this playlist has matching candidates on the destination service.
                </p>
                <button
                  type="button"
                  onClick={() => setActiveTab("all")}
                  className="px-3 py-1 rounded-md text-xs font-medium bg-white/[0.06] hover:bg-white/[0.1] text-zinc-300 transition-colors cursor-pointer"
                >
                  View All Tracks ({results.length})
                </button>
              </>
            ) : activeTab === "duplicates" ? (
              <>
                <p className="text-xs font-medium text-zinc-300 mb-1">
                  No Duplicate Tracks Found
                </p>
                <p className="text-[11px] text-zinc-500 mb-3 max-w-xs">
                  No recurring ISRC identifiers or duplicate title/artist pairs detected.
                </p>
                <button
                  type="button"
                  onClick={() => setActiveTab("all")}
                  className="px-3 py-1 rounded-md text-xs font-medium bg-white/[0.06] hover:bg-white/[0.1] text-zinc-300 transition-colors cursor-pointer"
                >
                  View All Tracks ({results.length})
                </button>
              </>
            ) : (
              <>
                <p className="text-xs font-medium text-zinc-300 mb-1">
                  No tracks match current filter criteria
                </p>
                <button
                  type="button"
                  onClick={() => {
                    setActiveTab("all");
                    setSearchQuery("");
                  }}
                  className="px-3 py-1 rounded-md text-xs font-medium bg-white/[0.06] hover:bg-white/[0.1] text-zinc-300 transition-colors cursor-pointer"
                >
                  Reset filters
                </button>
              </>
            )}
          </div>
        ) : (
          <div
            style={{
              height: `${rowVirtualizer.getTotalSize()}px`,
              width: "100%",
              position: "relative",
            }}
          >
            {rowVirtualizer.getVirtualItems().map((virtualRow) => {
              const result = filteredResults[virtualRow.index];
              const isSelected = selectedTrackIds.has(result.source_track.id);
              const isDuplicate = duplicateIdSet.has(result.source_track.id);

              return (
                <div
                  key={result.source_track.id}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    width: "100%",
                    height: `${virtualRow.size}px`,
                    transform: `translateY(${virtualRow.start}px)`,
                  }}
                >
                  <div className="w-full max-w-[1440px] mx-auto px-4 sm:px-6 h-full">
                    <DiffRow
                      result={result}
                      isSelected={isSelected}
                      isDuplicate={isDuplicate}
                      onToggleSelect={handleToggleSelect}
                      onOpenDrawer={handleOpenDrawer}
                    />
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* Sticky Bottom Action Bar */}
      <StagingActionBar
        selectedCount={selectedTrackIds.size}
        totalCount={results.length}
        exactCount={counts.exact}
        amberCount={counts.amber}
        redCount={counts.red}
        onAcceptAllAmbiguous={handleAcceptAllAmbiguous}
        onSkipUnresolved={handleSkipUnresolved}
        onDeduplicate={handleDeduplicate}
        onStartTransfer={() => setIsConfirmModalOpen(true)}
      />

      {/* Disambiguation Drawer */}
      <DisambiguationDrawer
        matchResult={activeDrawerTrack}
        targetService={targetService || "ytmusic"}
        isOpen={Boolean(activeDrawerTrack)}
        onClose={() => setActiveDrawerTrack(null)}
        onResolve={handleResolveCandidate}
      />

      {/* Pre-Transfer Confirmation Modal */}
      <TransferConfirmModal
        isOpen={isConfirmModalOpen}
        onClose={() => setIsConfirmModalOpen(false)}
        onConfirm={(targetOptions) => {
          setIsConfirmModalOpen(false);
          const selectedTracks = results.filter((r) => selectedTrackIds.has(r.source_track.id));
          onConfirmTransfer?.(selectedTracks, targetOptions);
        }}
        playlistTitle={playlistTitle}
        targetService={targetService || "ytmusic"}
        results={results}
        selectedTrackIds={selectedTrackIds}
      />
    </div>
  );
};
