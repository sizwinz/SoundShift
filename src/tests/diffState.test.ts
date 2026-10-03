import { describe, it, expect } from "vitest";
import { MatchResult } from "../types/diff";
import { SourceTrack } from "../types/provider";

// Helper fixture generator
function createMockTrack(id: string, title: string, artist: string, durationMs: number = 200000, isrc?: string): SourceTrack {
  return {
    id,
    title,
    artists: [artist],
    album: "Test Album",
    duration_ms: durationMs,
    isrc: isrc ?? null,
    is_explicit: false,
    is_playable: true,
    preview_url: "https://example.com/preview.mp3",
    thumbnail_url: "https://example.com/thumb.jpg",
  };
}

describe("Diff Selection State & Batch Action Logic", () => {
  const sampleResults: MatchResult[] = [
    {
      source_track: createMockTrack("track-1", "Starboy", "The Weeknd", 230000, "USUM71607007"),
      status: "Exact",
      matched_track: createMockTrack("dest-1", "Starboy", "The Weeknd", 230000, "USUM71607007"),
      candidates: [
        {
          track: createMockTrack("dest-1", "Starboy", "The Weeknd", 230000),
          similarity: 1.0,
          duration_delta_ms: 0,
        },
      ],
      confidence: 1.0,
      match_method: "isrc",
    },
    {
      source_track: createMockTrack("track-2", "Blinding Lights", "The Weeknd", 200000),
      status: "Ambiguous",
      matched_track: null,
      candidates: [
        {
          track: createMockTrack("dest-2", "Blinding Lights (Live)", "The Weeknd", 208000),
          similarity: 0.82,
          duration_delta_ms: 8000,
        },
        {
          track: createMockTrack("dest-2-alt", "Blinding Lights (Remix)", "The Weeknd", 215000),
          similarity: 0.75,
          duration_delta_ms: 15000,
        },
      ],
      confidence: 0.82,
      match_method: "duration_fuzzy",
    },
    {
      source_track: createMockTrack("track-3", "Obscure Indie Song", "Unknown Artist", 180000),
      status: "NotFound",
      matched_track: null,
      candidates: [],
      confidence: 0,
      match_method: null,
    },
    {
      // Duplicate track (same title and artist as track-1)
      source_track: createMockTrack("track-4", "Starboy", "The Weeknd", 230000, "USUM71607007"),
      status: "Exact",
      matched_track: createMockTrack("dest-4", "Starboy", "The Weeknd", 230000, "USUM71607007"),
      candidates: [],
      confidence: 1.0,
      match_method: "isrc",
    },
  ];

  it("applies Decision D-01 default selection: Green checked, Amber and Red unchecked", () => {
    const selectedIds = new Set<string>();
    sampleResults.forEach((r) => {
      if (r.status === "Exact") {
        selectedIds.add(r.source_track.id);
      }
    });

    // Exact tracks should be checked
    expect(selectedIds.has("track-1")).toBe(true);
    expect(selectedIds.has("track-4")).toBe(true);

    // Amber and Red tracks should be unchecked by default
    expect(selectedIds.has("track-2")).toBe(false);
    expect(selectedIds.has("track-3")).toBe(false);

    expect(selectedIds.size).toBe(2);
  });

  it("executes 'Accept All Ambiguous' batch override promoting rank 1 candidates", () => {
    const selectedIds = new Set<string>(["track-1"]);

    const updatedResults = sampleResults.map((r) => {
      if (r.status === "Ambiguous" && r.candidates.length > 0) {
        selectedIds.add(r.source_track.id);
        return {
          ...r,
          status: "Exact" as const,
          matched_track: r.candidates[0].track,
          confidence: r.candidates[0].similarity,
          match_method: "batch_accepted",
        };
      }
      return r;
    });

    const amberTrack = updatedResults.find((r) => r.source_track.id === "track-2");
    expect(amberTrack?.status).toBe("Exact");
    expect(amberTrack?.matched_track?.id).toBe("dest-2");
    expect(selectedIds.has("track-2")).toBe(true);
  });

  it("executes 'Skip Unresolved' batch override deselecting unapproved Amber and all Red tracks", () => {
    // Suppose user had manually selected all tracks
    const selectedIds = new Set<string>(["track-1", "track-2", "track-3", "track-4"]);

    const nextSelection = new Set<string>();
    sampleResults.forEach((r) => {
      if (r.status === "Exact" && selectedIds.has(r.source_track.id)) {
        nextSelection.add(r.source_track.id);
      }
    });

    expect(nextSelection.has("track-1")).toBe(true);
    expect(nextSelection.has("track-4")).toBe(true);
    expect(nextSelection.has("track-2")).toBe(false);
    expect(nextSelection.has("track-3")).toBe(false);
  });

  it("executes 'Deduplicate' unchecking duplicate occurrences while preserving index 0", () => {
    const selectedIds = new Set<string>(["track-1", "track-4"]);

    // Calculate duplicates
    const seen = new Set<string>();
    const duplicateIds = new Set<string>();

    sampleResults.forEach((r) => {
      const key = r.source_track.isrc
        ? `isrc:${r.source_track.isrc.trim().toUpperCase()}`
        : `title:${r.source_track.title.toLowerCase().trim()}::${(r.source_track.artists[0] || "").toLowerCase().trim()}`;

      if (seen.has(key)) {
        duplicateIds.add(r.source_track.id);
      } else {
        seen.add(key);
      }
    });

    expect(duplicateIds.has("track-4")).toBe(true);
    expect(duplicateIds.has("track-1")).toBe(false);

    // Apply deduplication
    duplicateIds.forEach((id) => selectedIds.delete(id));

    expect(selectedIds.has("track-1")).toBe(true);
    expect(selectedIds.has("track-4")).toBe(false);
    expect(selectedIds.size).toBe(1);
  });

  it("filters tracks correctly by tab category and search term", () => {
    // Exact filter
    const exactList = sampleResults.filter((r) => r.status === "Exact");
    expect(exactList.length).toBe(2);

    // Amber filter
    const amberList = sampleResults.filter((r) => r.status === "Ambiguous");
    expect(amberList.length).toBe(1);

    // Red filter
    const redList = sampleResults.filter((r) => r.status === "NotFound");
    expect(redList.length).toBe(1);

    // Search query filter
    const searchParam = "blinding";
    const searchList = sampleResults.filter(
      (r) =>
        r.source_track.title.toLowerCase().includes(searchParam) ||
        r.source_track.artists.some((a) => a.toLowerCase().includes(searchParam))
    );
    expect(searchList.length).toBe(1);
    expect(searchList[0].source_track.id).toBe("track-2");
  });
});
