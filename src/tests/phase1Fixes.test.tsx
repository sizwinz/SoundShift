// @vitest-environment jsdom
import indexCss from "../index.css?raw";
import { act } from "react";
import { createRoot, Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DisambiguationDrawer } from "../components/diff/DisambiguationDrawer";
import { AudioPlayButton } from "../components/diff/AudioPlayButton";
import { StagingTable } from "../components/diff/StagingTable";
import { SettingsPage } from "../components/settings/SettingsPage";
import { PlaylistSelector } from "../components/playlists/PlaylistSelector";
import { TransferProvider, useTransfer } from "../context/TransferContext";
import { AudioProvider } from "../context/AudioContext";
import { AuthProvider } from "../context/AuthContext";
import { MatchResult } from "../types/diff";
import { SourceTrack } from "../types/provider";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    async (name: string, callback: (event: { payload: unknown }) => void) => {
      mocks.listeners.set(name, callback);
      return () => mocks.listeners.delete(name);
    },
  ),
}));
vi.mock("../utils/tauri", () => ({ isTauri: () => true }));

function makeTrack(
  id: string,
  title: string,
  overrides: Partial<SourceTrack> = {},
): SourceTrack {
  return {
    id,
    title,
    artists: ["Artist"],
    album: "Album",
    duration_ms: 180_000,
    isrc: null,
    is_explicit: false,
    is_playable: true,
    preview_url: null,
    thumbnail_url: null,
    ...overrides,
  };
}

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
  (
    globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  mocks.invoke.mockReset();
  mocks.listeners.clear();
  localStorage.clear();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  localStorage.clear();
});

describe("Phase 1 bug fixes", () => {
  it("invokes resolve_track_by_url (not resolve_track_url) in DisambiguationDrawer and supports Save & Next", async () => {
    const first = makeTrack("src-1", "Track One");
    const second = makeTrack("src-2", "Track Two");
    const resolvedTrack = makeTrack("yt-1", "Track One YT");

    const results: MatchResult[] = [
      {
        source_track: first,
        status: "NotFound",
        matched_track: null,
        candidates: [],
        confidence: 0,
        match_method: null,
      },
      {
        source_track: second,
        status: "Ambiguous",
        matched_track: null,
        candidates: [],
        confidence: 0.72,
        match_method: "fuzzy",
      },
    ];

    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "resolve_track_by_url") return resolvedTrack;
      if (command === "save_manual_match_override") return null;
      throw new Error(`Unexpected command: ${command}`);
    });

    const onResolve = vi.fn();
    const onNavigateUnresolved = vi.fn();

    await act(async () => {
      root.render(
        <AudioProvider>
          <DisambiguationDrawer
            matchResult={results[0]}
            targetService="ytmusic"
            sourceService="spotify"
            isOpen
            onClose={() => {}}
            onResolve={onResolve}
            unresolvedPosition={1}
            totalUnresolved={2}
            onResolveAndNext={(id, track) => {
              onResolve(id, track);
              onNavigateUnresolved(1);
            }}
            onNavigateUnresolved={onNavigateUnresolved}
          />
        </AudioProvider>,
      );
    });

    const urlInput = container.querySelector<HTMLInputElement>('input[type="url"]')!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value",
      )!.set!;
      setter.call(urlInput, "https://music.youtube.com/watch?v=yt-1");
      urlInput.dispatchEvent(new Event("input", { bubbles: true }));
      urlInput.dispatchEvent(new Event("change", { bubbles: true }));
    });

    const form = urlInput.closest("form")!;
    await act(async () => {
      form.dispatchEvent(
        new Event("submit", { bubbles: true, cancelable: true }),
      );
    });

    expect(mocks.invoke).toHaveBeenCalledWith("resolve_track_by_url", {
      service: "ytmusic",
      url: "https://music.youtube.com/watch?v=yt-1",
    });

    const nextBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Next Unresolved"),
    );
    expect(nextBtn).toBeTruthy();
  });

  it("uses YouTube Music external link when AudioPlayButton service is ytmusic", async () => {
    const ytTrack = makeTrack("dQw4w9WgXcQ", "Video Track", { preview_url: null });
    await act(async () => {
      root.render(
        <AudioProvider>
          <AudioPlayButton track={ytTrack} service="ytmusic" />
        </AudioProvider>,
      );
    });
    const link = container.querySelector("a");
    expect(link?.getAttribute("href")).toBe(
      "https://music.youtube.com/watch?v=dQw4w9WgXcQ",
    );
  });

  it("handles duplicate same-ID tracks independently and keeps first copy when deduplicating", async () => {
    const sameTrack = makeTrack("same-id", "Repeated Song", { isrc: "US1234567890" });
    const unplayableTrack = makeTrack("dead-id", "Unavailable", { is_playable: false });
    const unresolvedTrack = makeTrack("amber-id", "Ambiguous Song");

    const rows: MatchResult[] = [
      {
        source_track: sameTrack,
        status: "Exact",
        matched_track: makeTrack("yt-same", "Repeated Song"),
        candidates: [],
        confidence: 1,
        match_method: "isrc",
      },
      {
        source_track: sameTrack,
        status: "Exact",
        matched_track: makeTrack("yt-same", "Repeated Song"),
        candidates: [],
        confidence: 1,
        match_method: "isrc",
      },
      {
        source_track: unplayableTrack,
        status: "Exact",
        matched_track: makeTrack("yt-dead", "Unavailable", { is_playable: false }),
        candidates: [],
        confidence: 1,
        match_method: "fuzzy",
      },
      {
        source_track: unresolvedTrack,
        status: "Ambiguous",
        matched_track: makeTrack("yt-amber", "Ambiguous Song"),
        candidates: [],
        confidence: 0.7,
        match_method: "fuzzy",
      },
    ];

    const consoleError = vi.spyOn(console, "error").mockImplementation(() => {});

    await act(async () => {
      root.render(
        <AudioProvider>
          <TransferProvider>
            <StagingTable
              results={rows}
              playlistTitle="Duplicates Test"
              sourceService="spotify"
              targetService="ytmusic"
            />
          </TransferProvider>
        </AudioProvider>,
      );
    });

    // Only the two playable Exact occurrences should start selected (not unplayable or Ambiguous)
    expect(container.textContent).toContain("2 of 4 selected");

    const dedupeBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Deduplicate"),
    )!;
    await act(async () => {
      dedupeBtn.click();
    });

    // First copy remains selected, second duplicate copy is deselected -> 1 of 4 selected
    expect(container.textContent).toContain("1 of 4 selected");

    // Disable confirm dialog and verify confirm passes exactly the first kept copy (not 2 copies or 0)
    localStorage.setItem(
      "soundshift.settings",
      JSON.stringify({ confirmBeforeTransfer: false }),
    );
    const onConfirmTransfer = vi.fn();

    await act(async () => {
      root.render(
        <AudioProvider>
          <TransferProvider>
            <StagingTable
              results={rows}
              playlistTitle="Duplicates Test"
              sourceService="spotify"
              targetService="ytmusic"
              onConfirmTransfer={onConfirmTransfer}
            />
          </TransferProvider>
        </AudioProvider>,
      );
    });

    const transferBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Transfer selected"),
    )!;
    const dedupeBtn2 = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Deduplicate"),
    )!;
    await act(async () => {
      dedupeBtn2.click();
    });
    await act(async () => {
      transferBtn.click();
    });

    expect(onConfirmTransfer).toHaveBeenCalledTimes(1);
    expect(onConfirmTransfer.mock.calls[0][0]).toHaveLength(1);
    expect(onConfirmTransfer.mock.calls[0][0][0].matched_track?.id).toBe("yt-same");
    expect(onConfirmTransfer.mock.calls[0][1].isNewPlaylist).toBe(true);

    const duplicateKeyErrors = consoleError.mock.calls.filter((args) =>
      String(args[0]).includes("same key"),
    );
    expect(duplicateKeyErrors).toHaveLength(0);
    consoleError.mockRestore();
  });

  it("does not nest interactive switch button inside label in SettingsPage", async () => {
    await act(async () => {
      root.render(<SettingsPage />);
    });
    const switches = Array.from(
      container.querySelectorAll('button[role="switch"]'),
    );
    expect(switches.length).toBeGreaterThan(0);
    for (const sw of switches) {
      expect(sw.closest("label")).toBeNull();
    }
  });

  it("respects keepTransferLogs=false in TransferContext", async () => {
    let api!: ReturnType<typeof useTransfer>;
    localStorage.setItem(
      "soundshift.settings",
      JSON.stringify({ keepTransferLogs: false }),
    );
    function Probe() {
      api = useTransfer();
      return null;
    }
    mocks.invoke.mockResolvedValue({
      job_id: "job_log_test",
      snapshot_id: "snap_1",
      target_playlist_id: "pl_1",
      total_tracks: 1,
      successful_tracks: 1,
      failed_tracks: 0,
      is_cancelled: false,
      audit: null,
      state: "completed_verified",
      event_seq: 1,
    });

    await act(async () => {
      root.render(
        <TransferProvider>
          <Probe />
        </TransferProvider>,
      );
    });

    await act(async () => {
      void api.startTransfer({
        job_id: "job_log_test",
        source_service: "spotify",
        target_service: "ytmusic",
        source_playlist_name: "Test",
        is_new_playlist: true,
        tracks: [],
        concurrency: 1,
      });
    });

    await act(async () => {
      mocks.listeners.get("transfer:log")?.({
        payload: {
          job_id: "job_log_test",
          timestamp: 1000,
          worker_id: 1,
          message: "Added track",
          level: "info",
        },
      });
    });

    expect(api.logs).toHaveLength(0);
  });

  it("uses non-fixed scoped footer and exposes public playlist URL input in PlaylistSelector", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "check_auth") {
        return { service: "spotify", status: "connected" };
      }
      if (command === "check_provider_connection") return null;
      if (command === "list_provider_playlists") {
        return [
          {
            id: "pl_1",
            service: "spotify",
            title: "My Mix",
            track_count: 10,
            is_public: false,
          },
        ];
      }
      return [];
    });

    await act(async () => {
      root.render(
        <AuthProvider>
          <PlaylistSelector onMatchComplete={() => {}} />
        </AuthProvider>,
      );
    });

    expect(container.innerHTML).not.toContain("fixed inset-x-0 bottom-0");
    const urlImportInput = container.querySelector<HTMLInputElement>(
      'input[aria-label="Import playlist URL or ID"]',
    );
    expect(urlImportInput).toBeTruthy();
  });

  it("has valid reduced-motion CSS syntax in src/index.css", () => {
    expect(indexCss).not.toContain(".reduce-motion *,\n@media");
    expect(indexCss).not.toContain(".reduce-motion *,\r\n@media");
    expect(indexCss).not.toContain("0.01ms");
  });
});
