// @vitest-environment jsdom
import { act } from "react";
import { createRoot, Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PlaylistSelector } from "../components/playlists/PlaylistSelector";
import { TransferConfirmModal } from "../components/diff/TransferConfirmModal";
import { StagingTable, occurrenceKey } from "../components/diff/StagingTable";
import { SyncPage } from "../components/sync/SyncPage";
import { AuthProvider } from "../context/AuthContext";
import { AudioProvider } from "../context/AudioContext";
import { TransferProvider } from "../context/TransferContext";
import { MatchResult } from "../types/diff";
import { SourceTrack } from "../types/provider";

function makeTrack(id: string, title: string): SourceTrack {
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
  };
}

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));
vi.mock("../utils/tauri", () => ({ isTauri: () => true }));

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
  (
    globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  mocks.invoke.mockReset();
  localStorage.clear();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => {
    root.unmount();
  });
  container.remove();
});

describe("Bugfix regressions", () => {
  it("aligns the swap direction button with the provider control rows in PlaylistSelector", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "check_auth") {
        return { service: "spotify", status: "disconnected" };
      }
      if (command === "check_provider_connection") return null;
      return [];
    });

    await act(async () => {
      root.render(
        <AuthProvider>
          <PlaylistSelector onMatchComplete={() => {}} />
        </AuthProvider>,
      );
    });

    const swapButton = container.querySelector<HTMLButtonElement>(
      'button[aria-label="Swap transfer direction"]',
    );
    expect(swapButton).toBeTruthy();
    expect(swapButton?.className).toContain("h-[42px]");
    expect(swapButton?.parentElement?.className).toContain("md:self-end");
  });

  it("enables and respects skipDuplicates in Create New Playlist mode on TransferConfirmModal", async () => {
    const onConfirm = vi.fn();
    const sameTarget = makeTrack("yt-shared", "Mangal Rang");
    const results: MatchResult[] = [
      {
        source_track: makeTrack("sp-1", "Mangal Rang (Album)"),
        status: "Exact",
        matched_track: sameTarget,
        candidates: [],
        confidence: 1,
        match_method: "isrc",
      },
      {
        source_track: makeTrack("sp-2", "Mangal Rang (Single)"),
        status: "Exact",
        matched_track: { ...sameTarget },
        candidates: [],
        confidence: 0.96,
        match_method: "fuzzy",
      },
    ];

    await act(async () => {
      root.render(
        <TransferConfirmModal
          isOpen={true}
          onClose={() => {}}
          onConfirm={onConfirm}
          playlistTitle="Ummm...."
          targetService="ytmusic"
          results={results}
          selectedTrackIds={new Set([occurrenceKey(results[0], 0), occurrenceKey(results[1], 1)])}
        />,
      );
    });

    const checkbox = document.body.querySelector<HTMLInputElement>('input[type="checkbox"]');
    expect(checkbox).toBeTruthy();
    expect(checkbox?.checked).toBe(true);

    const confirmBtn = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Confirm & Start Transfer"),
    );
    expect(confirmBtn).toBeTruthy();
    expect(confirmBtn?.disabled).toBe(false);

    await act(async () => {
      confirmBtn?.click();
    });

    expect(onConfirm).toHaveBeenCalledTimes(1);
    expect(onConfirm.mock.calls[0][0].isNewPlaylist).toBe(true);
    expect(onConfirm.mock.calls[0][0].skipDuplicates).toBe(true);
  });

  it("blocks confirming YouTube Music duplicate target matches if skipDuplicates is unchecked", async () => {
    const onConfirm = vi.fn();
    const sameTarget = makeTrack("yt-shared", "Mangal Rang");
    const results: MatchResult[] = [
      {
        source_track: makeTrack("sp-1", "Mangal Rang (Album)"),
        status: "Exact",
        matched_track: sameTarget,
        candidates: [],
        confidence: 1,
        match_method: "isrc",
      },
      {
        source_track: makeTrack("sp-2", "Mangal Rang (Single)"),
        status: "Exact",
        matched_track: { ...sameTarget },
        candidates: [],
        confidence: 0.96,
        match_method: "fuzzy",
      },
    ];

    await act(async () => {
      root.render(
        <TransferConfirmModal
          isOpen={true}
          onClose={() => {}}
          onConfirm={onConfirm}
          playlistTitle="Ummm...."
          targetService="ytmusic"
          results={results}
          selectedTrackIds={new Set([occurrenceKey(results[0], 0), occurrenceKey(results[1], 1)])}
        />,
      );
    });

    const checkbox = document.body.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
    const confirmBtn = Array.from(document.body.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Confirm & Start Transfer"),
    )!;
    expect(confirmBtn.disabled).toBe(false);

    await act(async () => {
      checkbox.click();
    });

    expect(checkbox.checked).toBe(false);
    expect(confirmBtn.disabled).toBe(true);
    expect(document.body.textContent).toContain("YouTube Music does not support keeping duplicate track occurrences");
  });

  it("flags colliding destination track IDs in StagingTable and defaults direct transfers to skipDuplicates=true", async () => {
    localStorage.setItem(
      "soundshift.settings",
      JSON.stringify({ confirmBeforeTransfer: false }),
    );
    const onConfirmTransfer = vi.fn();
    const sharedYt = makeTrack("yt-shared", "Mangal Rang");
    const results: MatchResult[] = [
      {
        source_track: { ...makeTrack("sp-1", "Mangal Rang (Album)"), isrc: "US111" },
        status: "Exact",
        matched_track: sharedYt,
        candidates: [],
        confidence: 1,
        match_method: "isrc",
      },
      {
        source_track: { ...makeTrack("sp-2", "Mangal Rang (Single)"), isrc: "US222" },
        status: "Exact",
        matched_track: { ...sharedYt },
        candidates: [],
        confidence: 0.96,
        match_method: "fuzzy",
      },
    ];

    await act(async () => {
      root.render(
        <AudioProvider>
          <TransferProvider>
            <StagingTable
              results={results}
              playlistTitle="Ummm...."
              sourceService="spotify"
              targetService="ytmusic"
              onConfirmTransfer={onConfirmTransfer}
            />
          </TransferProvider>
        </AudioProvider>,
      );
    });

    const dupTab = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Duplicates"),
    );
    expect(dupTab?.textContent).toContain("1");

    const transferBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Transfer selected"),
    );
    await act(async () => {
      transferBtn?.click();
    });

    expect(onConfirmTransfer).toHaveBeenCalledTimes(1);
    expect(onConfirmTransfer.mock.calls[0][1].skipDuplicates).toBe(true);
  });

  it("portals TransferConfirmModal outside #app-shell so opening the dialog from StagingTable never makes the modal inert", async () => {
    const onConfirmTransfer = vi.fn();
    const results: MatchResult[] = [
      {
        source_track: makeTrack("sp-1", "Song One"),
        status: "Exact",
        matched_track: makeTrack("yt-1", "Song One"),
        candidates: [],
        confidence: 1,
        match_method: "isrc",
      },
    ];

    await act(async () => {
      root.render(
        <div id="app-shell">
          <AudioProvider>
            <TransferProvider>
              <StagingTable
                results={results}
                playlistTitle="Ummm...."
                sourceService="spotify"
                targetService="ytmusic"
                onConfirmTransfer={onConfirmTransfer}
              />
            </TransferProvider>
          </AudioProvider>
        </div>,
      );
    });

    const appShell = container.querySelector<HTMLElement & { inert: boolean }>("#app-shell")!;
    const transferBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Transfer selected"),
    )!;
    await act(async () => {
      transferBtn.click();
    });

    const dialog = document.body.querySelector<HTMLElement>('[role="dialog"]');
    expect(dialog).toBeTruthy();
    expect(appShell.inert).toBe(true);
    expect(dialog?.closest("[inert]")).toBeNull();
    expect(appShell.contains(dialog)).toBe(false);

    const confirmBtn = Array.from(dialog!.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Confirm & Start Transfer"),
    )!;
    await act(async () => {
      confirmBtn.click();
    });

    expect(onConfirmTransfer).toHaveBeenCalledTimes(1);
    expect(appShell.inert).toBe(false);
  });

  it("uses cached playlists (refresh=false) when opening Link playlists in SyncPage", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_sync_pairs") return [];
      if (command === "list_provider_playlists") return [];
      return [];
    });

    await act(async () => {
      root.render(<SyncPage />);
    });

    const linkBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Link playlists"),
    )!;
    await act(async () => {
      linkBtn.click();
    });

    const listCalls = mocks.invoke.mock.calls.filter(
      ([command]) => command === "list_provider_playlists",
    );
    expect(listCalls).toHaveLength(2);
    expect(listCalls[0][1]).toEqual({ service: "spotify", refresh: false });
    expect(listCalls[1][1]).toEqual({ service: "ytmusic", refresh: false });
  });
});
