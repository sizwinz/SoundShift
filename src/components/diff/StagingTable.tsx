import React, { useRef, useMemo, useState, useEffect } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { MatchResult } from "../../types/diff";
import { SourceTrack } from "../../types/provider";
import { DiffRow } from "./DiffRow";
import { FilterToolbar, FilterTab, FilterCounts } from "./FilterToolbar";
import { DisambiguationDrawer } from "./DisambiguationDrawer";
import { StagingActionBar } from "./StagingActionBar";
import { TransferConfirmModal, TransferTargetOptions } from "./TransferConfirmModal";
import { readSoundShiftSettings } from "../settings/SettingsPage";
import { useAudioPreview } from "../../context/AudioContext";
import {
  downloadTextFile,
  exportMatchesToCsv,
  exportMatchesToJson,
  exportMatchesToM3u8,
} from "../../utils/playlistFormats";
import { Music, CheckCircle2, ArrowRight, ArrowLeft, Download } from "lucide-react";
import { Badge } from "../ui";

export function occurrenceKey(result: MatchResult, index: number): string {
  return `${result.source_track.id}::${index}`;
}

export function isRowSelectable(result: MatchResult): boolean {
  if (result.status !== "Exact" || !result.matched_track) return false;
  if (result.source_track.is_playable === false) return false;
  if (result.matched_track.is_playable === false) return false;
  return true;
}

interface IndexedMatchResult {
  key: string;
  index: number;
  result: MatchResult;
}

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
  const searchInputRef = useRef<HTMLInputElement>(null);
  const { activeTrackId } = useAudioPreview();

  const [activeTab, setActiveTab] = useState<FilterTab>("all");
  const [searchQuery, setSearchQuery] = useState("");
  const [activeDrawerKey, setActiveDrawerKey] = useState<string | null>(null);
  const [isConfirmModalOpen, setIsConfirmModalOpen] = useState(false);

  const playingTrackTitle = useMemo(() => {
    if (!activeTrackId) return undefined;
    const found = results.find(
      (r) => r.source_track.id === activeTrackId || r.matched_track?.id === activeTrackId
    );
    return found?.matched_track?.id === activeTrackId
      ? found.matched_track.title
      : found?.source_track.title;
  }, [activeTrackId, results]);

  const indexedResults: IndexedMatchResult[] = useMemo(
    () => results.map((result, index) => ({ key: occurrenceKey(result, index), index, result })),
    [results]
  );

  const datasetSignature = useMemo(
    () => `${playlistTitle ?? ""}|${indexedResults.map((item) => item.key).join(",")}`,
    [playlistTitle, indexedResults]
  );
  const prevSignatureRef = useRef<string>(datasetSignature);

  const [internalSelectedKeys, setInternalSelectedKeys] = useState<Set<string>>(() => {
    const initial = new Set<string>();
    indexedResults.forEach(({ key, result }) => {
      if (isRowSelectable(result)) {
        initial.add(key);
      }
    });
    return initial;
  });

  useEffect(() => {
    if (externalSelectedIds) return;
    if (prevSignatureRef.current !== datasetSignature) {
      prevSignatureRef.current = datasetSignature;
      const initial = new Set<string>();
      indexedResults.forEach(({ key, result }) => {
        if (isRowSelectable(result)) {
          initial.add(key);
        }
      });
      setInternalSelectedKeys(initial);
    }
  }, [datasetSignature, indexedResults, externalSelectedIds]);

  const effectiveSelectedKeys = useMemo(() => {
    const active = new Set<string>();
    indexedResults.forEach(({ key, result }) => {
      if (!isRowSelectable(result)) return;
      const chosen = externalSelectedIds
        ? externalSelectedIds.has(key) || externalSelectedIds.has(result.source_track.id)
        : internalSelectedKeys.has(key);
      if (chosen) {
        active.add(key);
      }
    });
    return active;
  }, [indexedResults, externalSelectedIds, internalSelectedKeys]);

  const updateSelection = (next: Set<string>) => {
    if (!externalSelectedIds) {
      setInternalSelectedKeys(next);
    }
    onSelectionChange?.(next);
  };

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);

  const duplicateKeySet = useMemo(() => {
    const seenSource = new Set<string>();
    const seenTargetIds = new Set<string>();
    const duplicates = new Set<string>();

    indexedResults.forEach(({ key, result }) => {
      const dedupeSignature = result.source_track.isrc
        ? `isrc:${result.source_track.isrc.trim().toUpperCase()}`
        : `title:${result.source_track.title.toLowerCase().trim()}::${(result.source_track.artists[0] || "").toLowerCase().trim()}`;
      const targetId = isRowSelectable(result) ? result.matched_track?.id : undefined;

      const isSourceDup = seenSource.has(dedupeSignature);
      const isTargetDup = Boolean(targetId && seenTargetIds.has(targetId));

      if (isSourceDup || isTargetDup) {
        duplicates.add(key);
      }
      seenSource.add(dedupeSignature);
      if (targetId) {
        seenTargetIds.add(targetId);
      }
    });

    return duplicates;
  }, [indexedResults]);

  const handleOpenDrawer = (occKey: string, result: MatchResult) => {
    setActiveDrawerKey(occKey);
    onOpenDrawer?.(result);
  };

  const handleResolveCandidate = (
    occKey: string | null,
    sourceTrackId: string,
    resolvedTrack: SourceTrack,
    advance: boolean
  ) => {
    const targetIdx = indexedResults.findIndex((item) =>
      occKey ? item.key === occKey : item.result.source_track.id === sourceTrackId
    );
    if (targetIdx === -1) return;

    const updatedResults = results.map((r, idx) => {
      if (idx === targetIdx) {
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

    const resolvedItem = indexedResults[targetIdx];
    const nextSelection = new Set(effectiveSelectedKeys);
    if (isRowSelectable(updatedResults[targetIdx])) {
      nextSelection.add(resolvedItem.key);
    }
    updateSelection(nextSelection);
    onUpdateResults?.(updatedResults);

    if (advance) {
      const nextUnresolved = indexedResults.find(
        (_item, idx) => idx > targetIdx && updatedResults[idx].status !== "Exact"
      ) ?? indexedResults.find(
        (_item, idx) => idx < targetIdx && updatedResults[idx].status !== "Exact"
      );
      setActiveDrawerKey(nextUnresolved ? nextUnresolved.key : null);
    } else {
      setActiveDrawerKey(null);
    }
  };

  const handleAcceptAllAmbiguous = () => {
    const nextSelection = new Set(effectiveSelectedKeys);
    const updated = results.map((r, idx) => {
      if (
        r.status === "Ambiguous" &&
        r.candidates.length > 0 &&
        r.candidates[0].similarity >= 0.85 &&
        r.candidates[0].duration_delta_ms <= 4_000
      ) {
        const top = r.candidates[0];
        const nextRow: MatchResult = {
          ...r,
          status: "Exact" as const,
          matched_track: top.track,
          confidence: top.similarity,
          match_method: "batch_accepted",
        };
        if (isRowSelectable(nextRow)) {
          nextSelection.add(occurrenceKey(r, idx));
        }
        return nextRow;
      }
      return r;
    });

    updateSelection(nextSelection);
    onUpdateResults?.(updated);
  };

  const handleSkipUnresolved = () => {
    const nextSelection = new Set<string>();
    indexedResults.forEach(({ key, result }) => {
      if (isRowSelectable(result) && effectiveSelectedKeys.has(key)) {
        nextSelection.add(key);
      }
    });
    updateSelection(nextSelection);
  };

  const handleDeduplicate = () => {
    const nextSelection = new Set(effectiveSelectedKeys);
    duplicateKeySet.forEach((key) => {
      nextSelection.delete(key);
    });
    updateSelection(nextSelection);
  };

  const counts: FilterCounts = useMemo(() => {
    let exact = 0;
    let amber = 0;
    let red = 0;
    let duplicates = 0;

    indexedResults.forEach(({ key, result }) => {
      if (result.status === "Exact") exact++;
      else if (result.status === "Ambiguous") amber++;
      else if (result.status === "NotFound") red++;

      if (duplicateKeySet.has(key)) {
        duplicates++;
      }
    });

    return {
      all: indexedResults.length,
      exact,
      amber,
      red,
      duplicates,
    };
  }, [indexedResults, duplicateKeySet]);

  const filteredResults = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();

    return indexedResults.filter(({ key, result }) => {
      if (activeTab === "exact" && result.status !== "Exact") return false;
      if (activeTab === "amber" && result.status !== "Ambiguous") return false;
      if (activeTab === "red" && result.status !== "NotFound") return false;
      if (activeTab === "duplicates" && !duplicateKeySet.has(key)) return false;

      if (query) {
        const titleMatch = result.source_track.title.toLowerCase().includes(query);
        const artistMatch = result.source_track.artists.some((a) =>
          a.toLowerCase().includes(query)
        );
        const destTitleMatch = result.matched_track?.title.toLowerCase().includes(query);
        const destArtistMatch = result.matched_track?.artists.some((a) =>
          a.toLowerCase().includes(query)
        );
        if (!titleMatch && !artistMatch && !destTitleMatch && !destArtistMatch) {
          return false;
        }
      }

      return true;
    });
  }, [indexedResults, activeTab, searchQuery, duplicateKeySet]);

  const rowVirtualizer = useVirtualizer({
    count: filteredResults.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 76,
    overscan: 10,
    initialRect: { width: 1024, height: 640 },
  });

  const handleToggleSelect = (occKey: string) => {
    const target = indexedResults.find((item) => item.key === occKey);
    if (!target) return;
    const next = new Set(effectiveSelectedKeys);
    if (next.has(occKey)) {
      next.delete(occKey);
    } else if (isRowSelectable(target.result)) {
      next.add(occKey);
    }
    updateSelection(next);
  };

  const selectableFiltered = useMemo(
    () => filteredResults.filter(({ result }) => isRowSelectable(result)),
    [filteredResults]
  );

  const isAllFilteredSelected = useMemo(() => {
    if (selectableFiltered.length === 0) return false;
    return selectableFiltered.every(({ key }) => effectiveSelectedKeys.has(key));
  }, [selectableFiltered, effectiveSelectedKeys]);

  const handleToggleSelectAllFiltered = () => {
    const next = new Set(effectiveSelectedKeys);
    if (isAllFilteredSelected) {
      selectableFiltered.forEach(({ key }) => next.delete(key));
    } else {
      selectableFiltered.forEach(({ key }) => next.add(key));
    }
    updateSelection(next);
  };

  const unresolvedItems = useMemo(
    () => indexedResults.filter((item) => item.result.status !== "Exact"),
    [indexedResults]
  );

  const activeDrawerItem = useMemo(
    () => (activeDrawerKey ? indexedResults.find((item) => item.key === activeDrawerKey) ?? null : null),
    [indexedResults, activeDrawerKey]
  );

  const activeUnresolvedIdx = useMemo(() => {
    if (!activeDrawerKey) return -1;
    return unresolvedItems.findIndex((item) => item.key === activeDrawerKey);
  }, [unresolvedItems, activeDrawerKey]);

  const handleNavigateUnresolved = (delta: -1 | 1) => {
    if (unresolvedItems.length === 0) return;
    if (activeUnresolvedIdx === -1) {
      setActiveDrawerKey(unresolvedItems[0].key);
      return;
    }
    const nextIdx =
      (activeUnresolvedIdx + delta + unresolvedItems.length) % unresolvedItems.length;
    setActiveDrawerKey(unresolvedItems[nextIdx].key);
  };

  const getSelectedMatchResults = () =>
    indexedResults
      .filter(({ key, result }) => effectiveSelectedKeys.has(key) && isRowSelectable(result))
      .map(({ result }) => result);

  const handleStartTransferClick = () => {
    const settings = readSoundShiftSettings();
    if (!settings.confirmBeforeTransfer) {
      onConfirmTransfer?.(getSelectedMatchResults(), {
        playlistName: playlistTitle || "Playlist Transfer",
        isNewPlaylist: true,
        targetPlaylistId: null,
        concurrency: 1,
        skipDuplicates: true,
      });
      return;
    }
    setIsConfirmModalOpen(true);
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
            <div className="hidden md:flex items-center gap-1 mr-1">
              <button
                type="button"
                aria-label="Export JSON"
                onClick={() =>
                  downloadTextFile(
                    `${(playlistTitle || "soundshift-staging").replace(/[^a-z0-9_-]+/gi, "_")}.json`,
                    exportMatchesToJson(
                      playlistTitle || "Playlist",
                      sourceService || "spotify",
                      targetService || "ytmusic",
                      results,
                    ),
                    "application/json",
                  )
                }
                className="inline-flex items-center gap-1 px-2 py-1 rounded-md text-[10px] font-mono text-zinc-400 hover:text-zinc-100 bg-white/[0.04] hover:bg-white/[0.08] border border-white/[0.06] transition-colors cursor-pointer"
                title="Export staged playlist & matches as JSON"
              >
                <Download className="w-3 h-3" />
                <span>JSON</span>
              </button>
              <button
                type="button"
                aria-label="Export CSV"
                onClick={() =>
                  downloadTextFile(
                    `${(playlistTitle || "soundshift-staging").replace(/[^a-z0-9_-]+/gi, "_")}.csv`,
                    exportMatchesToCsv(results),
                    "text/csv;charset=utf-8",
                  )
                }
                className="inline-flex items-center gap-1 px-2 py-1 rounded-md text-[10px] font-mono text-zinc-400 hover:text-zinc-100 bg-white/[0.04] hover:bg-white/[0.08] border border-white/[0.06] transition-colors cursor-pointer"
                title="Export staged playlist & matches as CSV"
              >
                <span>CSV</span>
              </button>
              <button
                type="button"
                aria-label="Export M3U8"
                onClick={() =>
                  downloadTextFile(
                    `${(playlistTitle || "soundshift-staging").replace(/[^a-z0-9_-]+/gi, "_")}.m3u8`,
                    exportMatchesToM3u8(
                      playlistTitle || "Playlist",
                      targetService || "ytmusic",
                      results,
                    ),
                    "application/x-mpegURL",
                  )
                }
                className="inline-flex items-center gap-1 px-2 py-1 rounded-md text-[10px] font-mono text-zinc-400 hover:text-zinc-100 bg-white/[0.04] hover:bg-white/[0.08] border border-white/[0.06] transition-colors cursor-pointer"
                title="Export matched tracks as M3U8 playlist"
              >
                <span>M3U8</span>
              </button>
            </div>
            <Badge variant="connected" icon={<CheckCircle2 className="w-3.5 h-3.5 text-emerald-400" />}>
              <span>{effectiveSelectedKeys.size} selected</span>
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
            searchInputRef={searchInputRef}
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
              const { key, result } = filteredResults[virtualRow.index];
              const isSelected = effectiveSelectedKeys.has(key);
              const isDuplicate = duplicateKeySet.has(key);

              return (
                <div
                  key={key}
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
                      occurrenceKey={key}
                      isSelected={isSelected}
                      isSelectable={isRowSelectable(result)}
                      isDuplicate={isDuplicate}
                      sourceService={sourceService}
                      targetService={targetService}
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
        selectedCount={effectiveSelectedKeys.size}
        totalCount={results.length}
        exactCount={counts.exact}
        amberCount={counts.amber}
        redCount={counts.red}
        playingTrackTitle={playingTrackTitle}
        onAcceptAllAmbiguous={handleAcceptAllAmbiguous}
        onSkipUnresolved={handleSkipUnresolved}
        onDeduplicate={handleDeduplicate}
        onStartTransfer={handleStartTransferClick}
      />

      {/* Disambiguation Drawer */}
      <DisambiguationDrawer
        matchResult={activeDrawerItem?.result ?? null}
        sourceService={sourceService}
        targetService={targetService || "ytmusic"}
        isOpen={Boolean(activeDrawerItem)}
        onClose={() => setActiveDrawerKey(null)}
        onResolve={(trackId, resolvedTrack) =>
          handleResolveCandidate(activeDrawerItem?.key ?? null, trackId, resolvedTrack, false)
        }
        unresolvedPosition={activeUnresolvedIdx >= 0 ? activeUnresolvedIdx + 1 : undefined}
        totalUnresolved={unresolvedItems.length}
        onResolveAndNext={(trackId, resolvedTrack) =>
          handleResolveCandidate(activeDrawerItem?.key ?? null, trackId, resolvedTrack, true)
        }
        onNavigateUnresolved={handleNavigateUnresolved}
      />

      {/* Pre-Transfer Confirmation Modal */}
      <TransferConfirmModal
        isOpen={isConfirmModalOpen}
        onClose={() => setIsConfirmModalOpen(false)}
        onConfirm={(targetOptions) => {
          setIsConfirmModalOpen(false);
          onConfirmTransfer?.(getSelectedMatchResults(), targetOptions);
        }}
        playlistTitle={playlistTitle}
        targetService={targetService || "ytmusic"}
        results={results}
        selectedTrackIds={effectiveSelectedKeys}
      />
    </div>
  );
};
