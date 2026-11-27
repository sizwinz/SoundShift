import { MatchResult } from "../types/diff";
import { SourceTrack } from "../types/provider";

export type PlaylistExportFormat = "json" | "csv" | "m3u8";

export interface ImportedPlaylistPayload {
  title: string;
  tracks: SourceTrack[];
}

function escapeCsvCell(value: string): string {
  if (/[",\r\n]/.test(value)) {
    return `"${value.replaceAll('"', '""')}"`;
  }
  return value;
}

export function exportMatchesToJson(
  playlistTitle: string,
  sourceService: string,
  targetService: string,
  results: MatchResult[]
): string {
  return JSON.stringify(
    {
      version: 1,
      app: "SoundShift",
      exported_at: new Date().toISOString(),
      playlist_title: playlistTitle,
      source_service: sourceService,
      target_service: targetService,
      track_count: results.length,
      tracks: results.map((r) => ({
        source: r.source_track,
        matched: r.matched_track,
        status: r.status,
        confidence: r.confidence,
        match_method: r.match_method,
      })),
    },
    null,
    2
  );
}

export function exportMatchesToCsv(results: MatchResult[]): string {
  const header = [
    "source_id",
    "source_title",
    "source_artists",
    "album",
    "duration_ms",
    "isrc",
    "status",
    "matched_id",
    "matched_title",
    "matched_artists",
    "confidence",
    "match_method",
  ].join(",");

  const rows = results.map((r) => {
    const cells = [
      r.source_track.id,
      r.source_track.title,
      r.source_track.artists.join("; "),
      r.source_track.album ?? "",
      String(r.source_track.duration_ms),
      r.source_track.isrc ?? "",
      r.status,
      r.matched_track?.id ?? "",
      r.matched_track?.title ?? "",
      r.matched_track?.artists.join("; ") ?? "",
      String(r.confidence),
      r.match_method ?? "",
    ];
    return cells.map(escapeCsvCell).join(",");
  });

  return [header, ...rows].join("\n");
}

export function exportMatchesToM3u8(
  playlistTitle: string,
  service: string,
  results: MatchResult[]
): string {
  const lines: string[] = ["#EXTM3U", `#PLAYLIST:${playlistTitle.replace(/[\r\n]/g, " ")}`];

  results.forEach((r) => {
    const track = r.matched_track ?? r.source_track;
    const secs = Math.max(0, Math.round((track.duration_ms || 0) / 1000));
    const artistLabel = track.artists.join(", ") || "Unknown Artist";
    lines.push(`#EXTINF:${secs},${artistLabel} - ${track.title}`);
    const urlService = r.matched_track ? service : "spotify";
    if (urlService === "ytmusic") {
      lines.push(`https://music.youtube.com/watch?v=${track.id}`);
    } else {
      lines.push(`https://open.spotify.com/track/${track.id}`);
    }
  });

  return `${lines.join("\n")}\n`;
}

function parseCsvRows(text: string): string[][] {
  const rows: string[][] = [];
  let row: string[] = [];
  let cell = "";
  let inQuotes = false;

  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    const next = text[i + 1];

    if (inQuotes) {
      if (ch === '"' && next === '"') {
        cell += '"';
        i++;
      } else if (ch === '"') {
        inQuotes = false;
      } else {
        cell += ch;
      }
      continue;
    }

    if (ch === '"') {
      inQuotes = true;
    } else if (ch === ",") {
      row.push(cell);
      cell = "";
    } else if (ch === "\r" && next === "\n") {
      row.push(cell);
      rows.push(row);
      row = [];
      cell = "";
      i++;
    } else if (ch === "\n" || ch === "\r") {
      row.push(cell);
      rows.push(row);
      row = [];
      cell = "";
    } else {
      cell += ch;
    }
  }

  if (cell.length > 0 || row.length > 0) {
    row.push(cell);
    rows.push(row);
  }

  return rows.filter((r) => r.some((c) => c.trim().length > 0));
}

function extractTrackIdFromUrl(line: string): string | null {
  const trimmed = line.trim();
  const spotifyTrack = trimmed.match(/open\.spotify\.com\/track\/([a-zA-Z0-9]+)/);
  if (spotifyTrack) return spotifyTrack[1];
  const spotifyUri = trimmed.match(/spotify:track:([a-zA-Z0-9]+)/);
  if (spotifyUri) return spotifyUri[1];
  const ytVideo = trimmed.match(/[?&]v=([a-zA-Z0-9_-]+)/);
  if (ytVideo) return ytVideo[1];
  return null;
}

function normalizeTrackObject(raw: Record<string, unknown>, index: number): SourceTrack | null {
  const candidate =
    (raw.source as Record<string, unknown> | undefined) ?? raw;
  const title =
    typeof candidate.title === "string" && candidate.title.trim()
      ? candidate.title.trim()
      : typeof candidate.name === "string" && candidate.name.trim()
      ? candidate.name.trim()
      : null;
  if (!title) return null;

  const artistsRaw = candidate.artists;
  const artists = Array.isArray(artistsRaw)
    ? artistsRaw.map((a) => String(a).trim()).filter(Boolean)
    : typeof candidate.artist === "string"
    ? candidate.artist
        .split(/[;,]/)
        .map((s) => s.trim())
        .filter(Boolean)
    : [];

  const durationMs =
    typeof candidate.duration_ms === "number" && Number.isFinite(candidate.duration_ms)
      ? Math.max(0, Math.round(candidate.duration_ms))
      : 0;

  return {
    id:
      typeof candidate.id === "string" && candidate.id.trim()
        ? candidate.id.trim()
        : `imported_${index}`,
    title,
    artists,
    album: typeof candidate.album === "string" ? candidate.album : null,
    duration_ms: durationMs,
    isrc: typeof candidate.isrc === "string" && candidate.isrc.trim() ? candidate.isrc.trim() : null,
    is_explicit: Boolean(candidate.is_explicit),
    is_playable: candidate.is_playable !== false,
    preview_url: typeof candidate.preview_url === "string" ? candidate.preview_url : null,
    thumbnail_url: typeof candidate.thumbnail_url === "string" ? candidate.thumbnail_url : null,
  };
}

export function importPlaylistContent(
  filename: string,
  rawContent: string
): ImportedPlaylistPayload {
  const trimmed = rawContent.trim();
  const baseTitle =
    filename.replace(/\.[^.]+$/, "").trim() || "Imported Playlist";

  if (!trimmed) {
    throw new Error("The selected file is empty.");
  }

  const lowerName = filename.toLowerCase();
  const isJson =
    lowerName.endsWith(".json") ||
    trimmed.startsWith("{") ||
    trimmed.startsWith("[");
  if (isJson) {
    const parsed = JSON.parse(trimmed) as unknown;
    const title =
      typeof parsed === "object" &&
      parsed !== null &&
      !Array.isArray(parsed) &&
      typeof (parsed as { playlist_title?: unknown }).playlist_title === "string"
        ? (parsed as { playlist_title: string }).playlist_title
        : baseTitle;

    const list: unknown[] = Array.isArray(parsed)
      ? parsed
      : typeof parsed === "object" &&
        parsed !== null &&
        Array.isArray((parsed as { tracks?: unknown }).tracks)
      ? (parsed as { tracks: unknown[] }).tracks
      : [];

    const tracks = list
      .map((item, idx) =>
        typeof item === "object" && item !== null
          ? normalizeTrackObject(item as Record<string, unknown>, idx)
          : null
      )
      .filter((t): t is SourceTrack => t !== null);

    if (tracks.length === 0) {
      throw new Error("No valid tracks found in JSON file.");
    }
    return { title, tracks };
  }

  const isM3u =
    lowerName.endsWith(".m3u8") ||
    lowerName.endsWith(".m3u") ||
    trimmed.startsWith("#EXTM3U") ||
    trimmed.includes("#EXTINF:");
  if (isM3u) {
    const lines = trimmed.split(/\r?\n/).map((l) => l.trim());
    let title = baseTitle;
    const tracks: SourceTrack[] = [];
    let pendingTitle: string | null = null;
    let pendingArtists: string[] = [];
    let pendingDurationMs = 0;

    for (const line of lines) {
      if (!line) continue;
      if (line.startsWith("#PLAYLIST:")) {
        const parsedTitle = line.slice("#PLAYLIST:".length).trim();
        if (parsedTitle) title = parsedTitle;
        continue;
      }
      if (line.startsWith("#EXTINF:")) {
        const rest = line.slice("#EXTINF:".length);
        const commaIdx = rest.indexOf(",");
        if (commaIdx !== -1) {
          const secs = Number.parseFloat(rest.slice(0, commaIdx));
          pendingDurationMs =
            Number.isFinite(secs) && secs > 0 ? Math.round(secs * 1000) : 0;
          const label = rest.slice(commaIdx + 1).trim();
          const dashIdx = label.indexOf(" - ");
          if (dashIdx !== -1) {
            pendingArtists = label
              .slice(0, dashIdx)
              .split(/[,;]/)
              .map((s) => s.trim())
              .filter(Boolean);
            pendingTitle = label.slice(dashIdx + 3).trim();
          } else {
            pendingArtists = [];
            pendingTitle = label;
          }
        }
        continue;
      }
      if (line.startsWith("#")) continue;

      const resolvedId = extractTrackIdFromUrl(line) ?? `imported_m3u_${tracks.length}`;
      const trackTitle = pendingTitle || line;
      tracks.push({
        id: resolvedId,
        title: trackTitle,
        artists: pendingArtists,
        album: null,
        duration_ms: pendingDurationMs,
        isrc: null,
        is_explicit: false,
        is_playable: true,
        preview_url: null,
        thumbnail_url: null,
      });
      pendingTitle = null;
      pendingArtists = [];
      pendingDurationMs = 0;
    }

    if (tracks.length === 0) {
      throw new Error("No valid tracks found in M3U/M3U8 playlist.");
    }
    return { title, tracks };
  }

  // Fallback: CSV parsing
  const rows = parseCsvRows(trimmed);
  if (rows.length < 2) {
    throw new Error("CSV file must include a header row and at least one track row.");
  }

  const headers = rows[0].map((h) => h.trim().toLowerCase());
  const col = (...candidates: string[]) =>
    headers.findIndex((h) => candidates.includes(h));

  const titleCol = col("source_title", "title", "track name", "name", "track");
  const artistsCol = col("source_artists", "artists", "artist", "artist name(s)", "artist name");
  const albumCol = col("album", "album name");
  const durationCol = col("duration_ms", "duration (ms)", "duration");
  const isrcCol = col("isrc");
  const idCol = col("source_id", "id", "track id", "spotify id");

  if (titleCol === -1) {
    throw new Error("CSV missing a 'title' or 'source_title' column.");
  }

  const tracks: SourceTrack[] = [];
  for (let r = 1; r < rows.length; r++) {
    const cells = rows[r];
    const trackTitle = (cells[titleCol] ?? "").trim();
    if (!trackTitle) continue;

    const rawArtists = artistsCol >= 0 ? cells[artistsCol] ?? "" : "";
    const artists = rawArtists
      .split(/[;,]/)
      .map((a) => a.trim())
      .filter(Boolean);

    const rawDur = durationCol >= 0 ? Number.parseFloat(cells[durationCol] ?? "") : 0;
    const duration_ms =
      Number.isFinite(rawDur) && rawDur > 0
        ? rawDur < 1000
          ? Math.round(rawDur * 1000)
          : Math.round(rawDur)
        : 0;

    tracks.push({
      id: (idCol >= 0 ? (cells[idCol] ?? "").trim() : "") || `imported_csv_${r}`,
      title: trackTitle,
      artists,
      album: albumCol >= 0 ? (cells[albumCol] ?? "").trim() || null : null,
      duration_ms,
      isrc: isrcCol >= 0 ? (cells[isrcCol] ?? "").trim() || null : null,
      is_explicit: false,
      is_playable: true,
      preview_url: null,
      thumbnail_url: null,
    });
  }

  if (tracks.length === 0) {
    throw new Error("No valid tracks found in CSV file.");
  }

  return { title: baseTitle, tracks };
}

export function downloadTextFile(filename: string, content: string, mimeType: string): void {
  const blob = new Blob([content], { type: mimeType });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  document.body.append(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}
