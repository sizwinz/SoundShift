import { describe, expect, it } from "vitest";
import {
  exportMatchesToCsv,
  exportMatchesToJson,
  exportMatchesToM3u8,
  importPlaylistContent,
} from "../utils/playlistFormats";
import { MatchResult } from "../types/diff";

const sampleResults: MatchResult[] = [
  {
    source_track: {
      id: "sp-1",
      title: "Midnight City, Remix",
      artists: ["M83", "Eric Prydz"],
      album: "Hurry Up",
      duration_ms: 243_000,
      isrc: "FRZ011100123",
      is_explicit: false,
      is_playable: true,
      preview_url: null,
      thumbnail_url: null,
    },
    status: "Exact",
    matched_track: {
      id: "yt-1",
      title: "Midnight City",
      artists: ["M83"],
      album: "Hurry Up",
      duration_ms: 244_000,
      isrc: null,
      is_explicit: false,
      is_playable: true,
      preview_url: null,
      thumbnail_url: null,
    },
    candidates: [],
    confidence: 1,
    match_method: "fuzzy",
  },
];

describe("playlistFormats export & import", () => {
  it("round-trips JSON playlist backups preserving tracks and title", () => {
    const json = exportMatchesToJson("Late Night", "spotify", "ytmusic", sampleResults);
    const imported = importPlaylistContent("backup.json", json);
    expect(imported.title).toBe("Late Night");
    expect(imported.tracks).toHaveLength(1);
    expect(imported.tracks[0].title).toBe("Midnight City, Remix");
    expect(imported.tracks[0].isrc).toBe("FRZ011100123");
  });

  it("escapes CSV cells with commas cleanly and imports back", () => {
    const csv = exportMatchesToCsv(sampleResults);
    expect(csv).toContain('"Midnight City, Remix"');
    const imported = importPlaylistContent("my_mix.csv", csv);
    expect(imported.title).toBe("my_mix");
    expect(imported.tracks[0].title).toBe("Midnight City, Remix");
    expect(imported.tracks[0].artists).toEqual(["M83", "Eric Prydz"]);
  });

  it("exports M3U8 playlists and parses #EXTINF + URL lines back cleanly", () => {
    const m3u = exportMatchesToM3u8("Roadtrip", "ytmusic", sampleResults);
    expect(m3u).toContain("#EXTM3U");
    expect(m3u).toContain("#PLAYLIST:Roadtrip");
    expect(m3u).toContain("https://music.youtube.com/watch?v=yt-1");

    const imported = importPlaylistContent("roadtrip.m3u8", m3u);
    expect(imported.title).toBe("Roadtrip");
    expect(imported.tracks).toHaveLength(1);
    expect(imported.tracks[0].id).toBe("yt-1");
    expect(imported.tracks[0].title).toBe("Midnight City");
    expect(imported.tracks[0].artists).toEqual(["M83"]);
  });
});
