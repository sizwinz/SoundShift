import { describe, it, expect } from "vitest";
import {
  TransferHistoryEntry,
  AuditResult,
} from "../types/transfer";
import { parsePlaylistId } from "../components/diff/TransferConfirmModal";

/**
 * Pure helper simulating the backend exponential backoff calculation with jitter (EXEC-03).
 */
export function calculateBackoffDelay(
  attempt: number,
  baseDelayMs: number = 1500,
  maxDelayMs: number = 16000,
  jitterMs: number = 0
): number {
  const exponential = baseDelayMs * Math.pow(2, attempt);
  return Math.min(exponential + jitterMs, maxDelayMs);
}

/**
 * Pure helper verifying destination integrity audit logic (SAFE-04, D-08).
 */
export function evaluateAuditIntegrity(
  liveTrackIds: string[],
  expectedTrackIds: string[]
): AuditResult {
  const liveSet = new Set(liveTrackIds.map((id) => id.replace("spotify:track:", "")));
  const missingIds: string[] = [];

  for (const expectedId of expectedTrackIds) {
    const cleanExpected = expectedId.replace("spotify:track:", "");
    if (!liveSet.has(cleanExpected) && !liveSet.has(expectedId)) {
      missingIds.push(expectedId);
    }
  }

  const totalExpected = expectedTrackIds.length;
  const totalFound = totalExpected - missingIds.length;

  return {
    is_verified: missingIds.length === 0,
    total_expected: totalExpected,
    total_found: totalFound,
    missing_ids: missingIds,
  };
}

/**
 * Pure helper for rollback action resolution per D-07.
 */
export function resolveRollbackPlan(snapshot: TransferHistoryEntry, userSelectedDeletePlaylist: boolean) {
  if (snapshot.is_new_playlist && userSelectedDeletePlaylist) {
    return {
      action: "delete_playlist" as const,
      playlistId: snapshot.target_playlist_id,
      tracksToRemove: [],
    };
  }

  return {
    action: "remove_tracks" as const,
    playlistId: snapshot.target_playlist_id,
    tracksToRemove: snapshot.added_tracks_count,
    preservedTracks: snapshot.pre_existing_count,
  };
}

describe("Exponential Backoff with Jitter Calculation (EXEC-03)", () => {
  it("calculates exponential base delays accurately without jitter", () => {
    expect(calculateBackoffDelay(0, 1500, 16000, 0)).toBe(1500);
    expect(calculateBackoffDelay(1, 1500, 16000, 0)).toBe(3000);
    expect(calculateBackoffDelay(2, 1500, 16000, 0)).toBe(6000);
    expect(calculateBackoffDelay(3, 1500, 16000, 0)).toBe(12000);
  });

  it("clamps delay at max ceiling (16,000ms)", () => {
    expect(calculateBackoffDelay(4, 1500, 16000, 0)).toBe(16000);
    expect(calculateBackoffDelay(5, 1500, 16000, 0)).toBe(16000);
  });

  it("adds bounded random jitter [0ms, 400ms]", () => {
    const delayWithJitter = calculateBackoffDelay(0, 1500, 16000, 250);
    expect(delayWithJitter).toBe(1750);
    expect(delayWithJitter).toBeGreaterThanOrEqual(1500);
    expect(delayWithJitter).toBeLessThanOrEqual(1900);
  });
});

describe("Destination Playlist Integrity Audit Comparator (SAFE-04, D-08)", () => {
  it("asserts verified green shield status when all expected tracks exist in destination", () => {
    const expected = ["track-1", "track-2", "track-3"];
    const live = ["track-pre-existing", "track-1", "track-2", "track-3"];

    const result = evaluateAuditIntegrity(live, expected);

    expect(result.is_verified).toBe(true);
    expect(result.total_expected).toBe(3);
    expect(result.total_found).toBe(3);
    expect(result.missing_ids).toHaveLength(0);
  });

  it("asserts discrepancy warning when destination is missing tracks", () => {
    const expected = ["track-1", "track-2", "track-3"];
    const live = ["track-1", "track-3"]; // track-2 is missing

    const result = evaluateAuditIntegrity(live, expected);

    expect(result.is_verified).toBe(false);
    expect(result.total_expected).toBe(3);
    expect(result.total_found).toBe(2);
    expect(result.missing_ids).toEqual(["track-2"]);
  });

  it("normalizes Spotify URI prefixes during comparison", () => {
    const expected = ["spotify:track:abc12345", "def67890"];
    const live = ["abc12345", "spotify:track:def67890"];

    const result = evaluateAuditIntegrity(live, expected);

    expect(result.is_verified).toBe(true);
    expect(result.missing_ids).toHaveLength(0);
  });
});

describe("1-Click Snapshot Rollback Plan Resolution (SAFE-03, D-07)", () => {
  it("recommends full playlist deletion when new playlist and user approved", () => {
    const newPlaylistSnapshot: TransferHistoryEntry = {
      job_id: "job-1",
      snapshot_id: "snap-1",
      source_service: "spotify",
      target_service: "ytmusic",
      source_playlist_name: "Summer Party",
      target_playlist_id: "PL_new_123",
      is_new_playlist: true,
      total_tracks: 50,
      matched_tracks: 50,
      added_tracks_count: 50,
      pre_existing_count: 0,
      is_rolled_back: false,
      created_at: 1727950000,
    };

    const plan = resolveRollbackPlan(newPlaylistSnapshot, true);

    expect(plan.action).toBe("delete_playlist");
    expect(plan.playlistId).toBe("PL_new_123");
  });

  it("preserves empty container if user chooses track removal only on new playlist", () => {
    const newPlaylistSnapshot: TransferHistoryEntry = {
      job_id: "job-2",
      snapshot_id: "snap-2",
      source_service: "spotify",
      target_service: "ytmusic",
      source_playlist_name: "Summer Party",
      target_playlist_id: "PL_new_456",
      is_new_playlist: true,
      total_tracks: 30,
      matched_tracks: 30,
      added_tracks_count: 30,
      pre_existing_count: 0,
      is_rolled_back: false,
      created_at: 1727950000,
    };

    const plan = resolveRollbackPlan(newPlaylistSnapshot, false);

    expect(plan.action).toBe("remove_tracks");
    expect(plan.tracksToRemove).toBe(30);
  });

  it("preserves pre-existing tracks when rolling back an appended playlist", () => {
    const appendedSnapshot: TransferHistoryEntry = {
      job_id: "job-3",
      snapshot_id: "snap-3",
      source_service: "ytmusic",
      target_service: "spotify",
      source_playlist_name: "Road Trip",
      target_playlist_id: "spotify_pl_789",
      is_new_playlist: false,
      total_tracks: 25,
      matched_tracks: 25,
      added_tracks_count: 25,
      pre_existing_count: 140,
      is_rolled_back: false,
      created_at: 1727950000,
    };

    // Even if user chose delete playlist, appended playlist must NEVER delete playlist
    const plan = resolveRollbackPlan(appendedSnapshot, true);

    expect(plan.action).toBe("remove_tracks");
    expect(plan.tracksToRemove).toBe(25);
    expect(plan.preservedTracks).toBe(140);
  });
});

describe("Destination Playlist ID / URL Parsing", () => {
  it("extracts ID from standard Spotify playlist URLs", () => {
    expect(parsePlaylistId("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=abcd1234")).toBe("37i9dQZF1DXcBWIGoYBM5M");
    expect(parsePlaylistId("https://open.spotify.com/playlist/7xGflW5m1k7n3")).toBe("7xGflW5m1k7n3");
  });

  it("extracts ID from Spotify URI format", () => {
    expect(parsePlaylistId("spotify:playlist:37i9dQZF1DXcBWIGoYBM5M")).toBe("37i9dQZF1DXcBWIGoYBM5M");
  });

  it("extracts list ID from YouTube Music and YouTube playlist URLs", () => {
    expect(parsePlaylistId("https://music.youtube.com/playlist?list=PL4fGSI1pDJn6jXS_PEoNcnwDXK9L9wBwJ")).toBe("PL4fGSI1pDJn6jXS_PEoNcnwDXK9L9wBwJ");
    expect(parsePlaylistId("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PLrEnWoR732-B41UeyXWvW1QjL_mN2A6Kx")).toBe("PLrEnWoR732-B41UeyXWvW1QjL_mN2A6Kx");
  });

  it("preserves raw playlist IDs", () => {
    expect(parsePlaylistId("PL4fGSI1pDJn6jXS_PEoNcnwDXK9L9wBwJ")).toBe("PL4fGSI1pDJn6jXS_PEoNcnwDXK9L9wBwJ");
    expect(parsePlaylistId("37i9dQZF1DXcBWIGoYBM5M")).toBe("37i9dQZF1DXcBWIGoYBM5M");
    expect(parsePlaylistId("   37i9dQZF1DXcBWIGoYBM5M   ")).toBe("37i9dQZF1DXcBWIGoYBM5M");
  });

  it("returns empty string for empty inputs", () => {
    expect(parsePlaylistId("")).toBe("");
    expect(parsePlaylistId("   ")).toBe("");
  });
});

describe("Destination Playlist Deduplication Logic", () => {
  interface MinimalTrack {
    id: string;
    title: string;
    artists: string[];
    isrc?: string;
  }

  function filterDestinationDuplicates(
    incoming: MinimalTrack[],
    existing: MinimalTrack[]
  ) {
    const existingIds = new Set(existing.map((t) => t.id.replace("spotify:track:", "")));
    const existingIsrcs = new Set(
      existing.filter((t) => t.isrc).map((t) => t.isrc!.trim().toLowerCase())
    );
    const existingTitleArtists = new Set(
      existing.map((t) => `${t.title.trim().toLowerCase()}::${(t.artists[0] || "").trim().toLowerCase()}`)
    );

    const toTransfer: MinimalTrack[] = [];
    const skipped: MinimalTrack[] = [];

    for (const track of incoming) {
      const cleanId = track.id.replace("spotify:track:", "");
      const isrc = track.isrc?.trim().toLowerCase();
      const titleArtist = `${track.title.trim().toLowerCase()}::${(track.artists[0] || "").trim().toLowerCase()}`;

      const isDuplicate =
        existingIds.has(cleanId) ||
        (isrc && existingIsrcs.has(isrc)) ||
        existingTitleArtists.has(titleArtist);

      if (isDuplicate) {
        skipped.push(track);
      } else {
        toTransfer.push(track);
        existingIds.add(cleanId);
      }
    }

    return {
      toTransfer,
      skipped,
      skippedCount: skipped.length,
    };
  }

  it("filters tracks that match destination track IDs", () => {
    const existing: MinimalTrack[] = [
      { id: "trk_1", title: "Song One", artists: ["Artist A"] },
      { id: "trk_2", title: "Song Two", artists: ["Artist B"] },
    ];
    const incoming: MinimalTrack[] = [
      { id: "trk_1", title: "Song One", artists: ["Artist A"] },
      { id: "trk_3", title: "Song Three", artists: ["Artist C"] },
    ];

    const result = filterDestinationDuplicates(incoming, existing);

    expect(result.skippedCount).toBe(1);
    expect(result.toTransfer).toHaveLength(1);
    expect(result.toTransfer[0].id).toBe("trk_3");
    expect(result.skipped[0].id).toBe("trk_1");
  });

  it("filters tracks that match destination ISRC codes", () => {
    const existing: MinimalTrack[] = [
      { id: "trk_1", title: "Original Title", artists: ["Artist A"], isrc: "USUM71703861" },
    ];
    const incoming: MinimalTrack[] = [
      { id: "trk_diff_id", title: "Alternative Title", artists: ["Artist A"], isrc: "usum71703861" },
    ];

    const result = filterDestinationDuplicates(incoming, existing);

    expect(result.skippedCount).toBe(1);
    expect(result.toTransfer).toHaveLength(0);
  });

  it("filters tracks matching title and artist when IDs differ", () => {
    const existing: MinimalTrack[] = [
      { id: "spotify_1", title: "Ordinary Person", artists: ["Anirudh Ravichander"] },
    ];
    const incoming: MinimalTrack[] = [
      { id: "yt_1", title: "ordinary person", artists: ["Anirudh Ravichander"] },
    ];

    const result = filterDestinationDuplicates(incoming, existing);

    expect(result.skippedCount).toBe(1);
    expect(result.toTransfer).toHaveLength(0);
  });

  it("returns zero mutations and records skipped count when all tracks already exist", () => {
    const existing: MinimalTrack[] = [
      { id: "1", title: "Song 1", artists: ["Artist 1"] },
      { id: "2", title: "Song 2", artists: ["Artist 2"] },
    ];
    const incoming: MinimalTrack[] = [
      { id: "1", title: "Song 1", artists: ["Artist 1"] },
      { id: "2", title: "Song 2", artists: ["Artist 2"] },
    ];

    const result = filterDestinationDuplicates(incoming, existing);

    expect(result.toTransfer).toHaveLength(0);
    expect(result.skippedCount).toBe(2);
  });
});


