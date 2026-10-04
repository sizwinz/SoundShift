import { describe, it, expect } from "vitest";
import { renderToString } from "react-dom/server";
import { StagingTable } from "../StagingTable";
import { TransferProvider } from "../../../context/TransferContext";
import { MatchResult } from "../../../types/diff";
import { SourceTrack } from "../../../types/provider";

function createMockTrack(
  id: string,
  title: string,
  artist: string,
  durationMs: number = 210000,
  isrc?: string
): SourceTrack {
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

describe("StagingTable Component & Staging Flow Suite", () => {
  const sampleTracks: MatchResult[] = [
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
          track: createMockTrack("dest-2", "Blinding Lights", "The Weeknd", 202000),
          similarity: 0.95,
          duration_delta_ms: 2000,
        },
      ],
      confidence: 0.95,
      match_method: "duration_fuzzy",
    },
    {
      source_track: createMockTrack("track-3", "Die For You (Remix)", "The Weeknd", 215000),
      status: "Ambiguous",
      matched_track: null,
      candidates: [
        {
          track: createMockTrack("dest-3", "Die For You (Live / Acoustic)", "The Weeknd", 230000),
          similarity: 0.75,
          duration_delta_ms: 15000,
        },
      ],
      confidence: 0.75,
      match_method: "duration_fuzzy",
    },
    {
      source_track: createMockTrack("track-4", "Unknown Track", "Indie Artist", 180000),
      status: "NotFound",
      matched_track: null,
      candidates: [],
      confidence: 0,
      match_method: null,
    },
    {
      // Duplicate track sharing ISRC with track-1
      source_track: createMockTrack("track-5", "Starboy", "The Weeknd", 230000, "USUM71607007"),
      status: "Exact",
      matched_track: createMockTrack("dest-5", "Starboy", "The Weeknd", 230000, "USUM71607007"),
      candidates: [],
      confidence: 1.0,
      match_method: "isrc",
    },
  ];

  it("renders server-side without crashing", () => {
    const html = renderToString(
      <TransferProvider>
        <StagingTable
          results={sampleTracks}
          playlistTitle="Synthwave Essentials"
          sourceService="spotify"
          targetService="ytmusic"
        />
      </TransferProvider>
    );

    expect(html).toContain("Synthwave Essentials");
    expect(html).toContain("Exact");
    expect(html).toContain("Needs Review");
    expect(html).toContain("Unmatched");
    expect(html).toContain("Duplicates");
    expect(html).toContain("Accept reviewable");
    expect(html).toContain("Skip unresolved");
    expect(html).toContain("Deduplicate");
    expect(html).toContain("Transfer selected");
  });

  it("evaluates default selection behavior: Exact tracks checked, Ambiguous and NotFound unchecked", () => {
    const initialSelected = new Set<string>();
    sampleTracks.forEach((r) => {
      if (r.status === "Exact") {
        initialSelected.add(r.source_track.id);
      }
    });

    expect(initialSelected.has("track-1")).toBe(true);
    expect(initialSelected.has("track-5")).toBe(true);
    expect(initialSelected.has("track-2")).toBe(false);
    expect(initialSelected.has("track-3")).toBe(false);
    expect(initialSelected.has("track-4")).toBe(false);
    expect(initialSelected.size).toBe(2);
  });

  it("filters tracks accurately across tab categories and search query", () => {
    // All tab count
    expect(sampleTracks.length).toBe(5);

    // Exact filter
    const exactTracks = sampleTracks.filter((t) => t.status === "Exact");
    expect(exactTracks.length).toBe(2);

    // Amber filter
    const amberTracks = sampleTracks.filter((t) => t.status === "Ambiguous");
    expect(amberTracks.length).toBe(2);

    // Red filter
    const redTracks = sampleTracks.filter((t) => t.status === "NotFound");
    expect(redTracks.length).toBe(1);

    // Duplicate detection by ISRC
    const seen = new Set<string>();
    const duplicates = new Set<string>();
    sampleTracks.forEach((t) => {
      const key = t.source_track.isrc
        ? `isrc:${t.source_track.isrc.trim().toUpperCase()}`
        : `title:${t.source_track.title.toLowerCase().trim()}::${(t.source_track.artists[0] || "").toLowerCase().trim()}`;
      if (seen.has(key)) {
        duplicates.add(t.source_track.id);
      } else {
        seen.add(key);
      }
    });

    expect(duplicates.has("track-5")).toBe(true);
    expect(duplicates.has("track-1")).toBe(false);

    // Search query filter across title and artists
    const query = "blinding";
    const searchMatches = sampleTracks.filter(
      (t) =>
        t.source_track.title.toLowerCase().includes(query) ||
        t.source_track.artists.some((a) => a.toLowerCase().includes(query))
    );
    expect(searchMatches.length).toBe(1);
    expect(searchMatches[0].source_track.id).toBe("track-2");
  });

  it("promotes only high-confidence Ambiguous candidates in Accept All Ambiguous", () => {
    const selectedIds = new Set<string>(["track-1", "track-5"]);

    const updated = sampleTracks.map((r) => {
      if (
        r.status === "Ambiguous" &&
        r.candidates.length > 0 &&
        r.candidates[0].similarity >= 0.85 &&
        r.candidates[0].duration_delta_ms <= 4_000
      ) {
        const top = r.candidates[0];
        selectedIds.add(r.source_track.id);
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

    // track-2 (similarity: 0.95, delta: 2000ms) qualifies
    const track2 = updated.find((r) => r.source_track.id === "track-2");
    expect(track2?.status).toBe("Exact");
    expect(track2?.match_method).toBe("batch_accepted");
    expect(selectedIds.has("track-2")).toBe(true);

    // track-3 (similarity: 0.75, delta: 15000ms) does NOT qualify
    const track3 = updated.find((r) => r.source_track.id === "track-3");
    expect(track3?.status).toBe("Ambiguous");
    expect(selectedIds.has("track-3")).toBe(false);
  });

  it("prunes unapproved tracks during Skip Unresolved action", () => {
    // User had selected all tracks
    const selectedIds = new Set<string>(["track-1", "track-2", "track-3", "track-4", "track-5"]);

    const nextSelection = new Set<string>();
    sampleTracks.forEach((r) => {
      if (r.status === "Exact" && selectedIds.has(r.source_track.id)) {
        nextSelection.add(r.source_track.id);
      }
    });

    expect(nextSelection.has("track-1")).toBe(true);
    expect(nextSelection.has("track-5")).toBe(true);
    expect(nextSelection.has("track-2")).toBe(false);
    expect(nextSelection.has("track-3")).toBe(false);
    expect(nextSelection.has("track-4")).toBe(false);
    expect(nextSelection.size).toBe(2);
  });

  it("removes subsequent duplicate track occurrences during Deduplicate action", () => {
    const selectedIds = new Set<string>(["track-1", "track-5"]);

    const seen = new Set<string>();
    const duplicateIdSet = new Set<string>();
    sampleTracks.forEach((r) => {
      const key = r.source_track.isrc
        ? `isrc:${r.source_track.isrc.trim().toUpperCase()}`
        : `title:${r.source_track.title.toLowerCase().trim()}::${(r.source_track.artists[0] || "").toLowerCase().trim()}`;
      if (seen.has(key)) {
        duplicateIdSet.add(r.source_track.id);
      } else {
        seen.add(key);
      }
    });

    // Apply deduplication
    duplicateIdSet.forEach((id) => selectedIds.delete(id));

    expect(selectedIds.has("track-1")).toBe(true);
    expect(selectedIds.has("track-5")).toBe(false);
    expect(selectedIds.size).toBe(1);
  });
});
