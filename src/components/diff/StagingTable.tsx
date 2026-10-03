import React, { useRef, useMemo, useState, useEffect } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { MatchResult } from "../../types/diff";
import { SourceTrack } from "../../types/provider";
import { DiffRow } from "./DiffRow";
import { FilterToolbar, FilterTab, FilterCounts } from "./FilterToolbar";
import { DisambiguationDrawer } from "./DisambiguationDrawer";
import { StagingActionBar } from "./StagingActionBar";
import { TransferConfirmModal } from "./TransferConfirmModal";
import { Music, CheckCircle2 } from "lucide-react";

interface StagingTableProps {
  results: MatchResult[];
  onUpdateResults?: (newResults: MatchResult[]) => void;
  onOpenDrawer?: (result: MatchResult) => void;
  onConfirmTransfer?: (selectedTracks: MatchResult[]) => void;
  selectedTrackIds?: Set<string>;
  onSelectionChange?: (selectedIds: Set<string>) => void;
  playlistTitle?: string;
  sourceService?: string;
  targetService?: string;
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
}) => {
  const parentRef = useRef<HTMLDivElement>(null);

  // Filter & Search state
  const [activeTab, setActiveTab] = useState<FilterTab>("all");
  const [searchQuery, setSearchQuery] = useState("");

  // Drawer state for Amber and Red row disambiguation per DIFF-03, D-05
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

  // Internal selection state if not externally controlled
  const [internalSelectedIds, setInternalSelectedIds] = useState<Set<string>>(() => {
    // Decision D-01: Green (Exact) checked by default; Amber/Red unchecked by default
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

  // Pre-transfer confirmation modal state per D-13
  const [isConfirmModalOpen, setIsConfirmModalOpen] = useState(false);

  // Batch action 1: Accept All Ambiguous per DIFF-06 and D-12
  const handleAcceptAllAmbiguous = () => {
    const nextSelection = new Set(selectedTrackIds);
    const updated = results.map((r) => {
      if (r.status === "Ambiguous" && r.candidates.length > 0) {
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

  // Batch action 2: Skip Unresolved per DIFF-06 and D-12
  const handleSkipUnresolved = () => {
    const nextSelection = new Set<string>();
    results.forEach((r) => {
      if (r.status === "Exact" && selectedTrackIds.has(r.source_track.id)) {
        nextSelection.add(r.source_track.id);
      }
    });
    updateSelection(nextSelection);
  };

  // Batch action 3: Deduplicate per D-12
  const handleDeduplicate = () => {
    const nextSelection = new Set(selectedTrackIds);
    duplicateIdSet.forEach((id) => {
      nextSelection.delete(id);
    });
    updateSelection(nextSelection);
  };

  // Synchronize initial selection if results array changes
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

  // Identify duplicate track occurrences
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

  // Compute live filter counts
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

  // Apply tab filter & search query
  const filteredResults = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();

    return results.filter((r) => {
      // Tab filter
      if (activeTab === "exact" && r.status !== "Exact") return false;
      if (activeTab === "amber" && r.status !== "Ambiguous") return false;
      if (activeTab === "red" && r.status !== "NotFound") return false;
      if (activeTab === "duplicates" && !duplicateIdSet.has(r.source_track.id)) return false;

      // Text search filter
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

  // Virtualizer configuration: fixed 56px row height per D-04
  const rowVirtualizer = useVirtualizer({
    count: filteredResults.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 56,
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
    <div className="flex flex-col h-full bg-[#000000] border border-[#27272a] rounded-lg overflow-hidden shadow-2xl">
      {/* Header bar */}
      <div className="flex items-center justify-between px-4 py-3 bg-[#09090b] border-b border-[#27272a]">
        <div className="flex items-center gap-3">
          <div className="w-8 h-8 rounded bg-zinc-800 flex items-center justify-center text-zinc-400">
            <Music className="w-4 h-4" />
          </div>
          <div>
            <h2 className="text-sm font-semibold text-zinc-100 flex items-center gap-2">
              <span>{playlistTitle || "Playlist Staging Diff"}</span>
              <span className="text-xs font-normal text-zinc-500">
                ({results.length} total tracks)
              </span>
            </h2>
            <div className="text-[11px] text-zinc-500">
              Source: <span className="capitalize text-zinc-300">{sourceService || "Unknown"}</span>
              {" -> "}
              Target: <span className="capitalize text-zinc-300">{targetService || "Unknown"}</span>
            </div>
          </div>
        </div>

        <div className="flex items-center gap-2">
          <span className="inline-flex items-center gap-1 px-2.5 py-1 rounded bg-zinc-900 border border-[#27272a] text-xs font-mono text-zinc-300">
            <CheckCircle2 className="w-3.5 h-3.5 text-emerald-400" />
            <span>{selectedTrackIds.size} selected</span>
          </span>
        </div>
      </div>

      {/* Filter Toolbar */}
      <FilterToolbar
        activeTab={activeTab}
        onTabChange={setActiveTab}
        searchQuery={searchQuery}
        onSearchChange={setSearchQuery}
        counts={counts}
        isAllFilteredSelected={isAllFilteredSelected}
        onToggleSelectAllFiltered={handleToggleSelectAllFiltered}
      />

      {/* Column Headers */}
      <div className="h-8 flex items-center px-4 bg-[#09090b] border-b border-[#18181b] text-[11px] font-medium text-zinc-500 uppercase tracking-wider select-none">
        <div className="w-7 mr-3"></div>
        <div className="flex-1">Source Track</div>
        <div className="w-36 text-center">Match Status</div>
        <div className="flex-1 text-right">Destination Track</div>
      </div>

      {/* Virtualized List Container */}
      <div
        ref={parentRef}
        className="flex-1 overflow-y-auto scrollbar-thin scrollbar-thumb-zinc-800 scrollbar-track-transparent relative min-h-[400px] max-h-[calc(100vh-280px)]"
      >
        {filteredResults.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-48 text-zinc-500">
            <Music className="w-8 h-8 mb-2 opacity-30" />
            <p className="text-xs">No tracks match current filter criteria</p>
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
                  <DiffRow
                    result={result}
                    isSelected={isSelected}
                    isDuplicate={isDuplicate}
                    onToggleSelect={handleToggleSelect}
                    onOpenDrawer={handleOpenDrawer}
                  />
                </div>
              );
            })}
          </div>
        )}
      </div>

      {/* Sticky Bottom Action Bar per DIFF-06, D-11, D-12 */}
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

      {/* Disambiguation Drawer for Amber and Red tracks per DIFF-03, DIFF-05, D-05 */}
      <DisambiguationDrawer
        matchResult={activeDrawerTrack}
        targetService={targetService || "ytmusic"}
        isOpen={Boolean(activeDrawerTrack)}
        onClose={() => setActiveDrawerTrack(null)}
        onResolve={handleResolveCandidate}
      />

      {/* Pre-Transfer Confirmation Modal per D-13, FR-4.1, EXEC-01 */}
      <TransferConfirmModal
        isOpen={isConfirmModalOpen}
        onClose={() => setIsConfirmModalOpen(false)}
        onConfirm={() => {
          setIsConfirmModalOpen(false);
          const selectedTracks = results.filter((r) => selectedTrackIds.has(r.source_track.id));
          onConfirmTransfer?.(selectedTracks);
        }}
        playlistTitle={playlistTitle}
        targetService={targetService || "ytmusic"}
        results={results}
        selectedTrackIds={selectedTrackIds}
      />
    </div>
  );
};
