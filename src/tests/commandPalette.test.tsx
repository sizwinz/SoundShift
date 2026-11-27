// @vitest-environment jsdom
import { act } from "react";
import { createRoot, Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "../App";
import { PlaylistSelector } from "../components/playlists/PlaylistSelector";
import { StagingTable } from "../components/diff/StagingTable";
import { AuthProvider } from "../context/AuthContext";
import { AudioProvider } from "../context/AudioContext";
import { TransferProvider } from "../context/TransferContext";
import { MatchResult } from "../types/diff";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isMaximized: async () => false,
    onResized: async () => () => {},
    minimize: async () => {},
    toggleMaximize: async () => {},
    close: async () => {},
  }),
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
  sessionStorage.clear();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe("Phase 4 Command Palette, Exports & Shortcuts", () => {
  it("opens Ctrl+K command palette, switches to Settings on Enter, and syncs tray preference on boot", async () => {
    localStorage.setItem(
      "soundshift.settings",
      JSON.stringify({ minimizeToTray: true }),
    );
    mocks.invoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_auth") return { service: "spotify", status: "disconnected" };
      if (cmd === "set_tray_preference") return null;
      return [];
    });

    await act(async () => {
      root.render(<App />);
    });

    expect(mocks.invoke).toHaveBeenCalledWith("set_tray_preference", { enabled: true });

    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "k", ctrlKey: true }));
    });

    const dialog = document.querySelector('[role="dialog"][aria-label="Command Palette"]');
    expect(dialog).toBeTruthy();

    const input = document.querySelector<HTMLInputElement>(
      'input[aria-label="Command search"]',
    )!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(input, "Settings");
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });

    await act(async () => {
      dialog!.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });

    expect(document.querySelector('[role="dialog"][aria-label="Command Palette"]')).toBeNull();
    expect(container.textContent).toContain("Minimize to system tray on close");
  });

  it("exports staged tracks to JSON, CSV, and M3U8 cleanly from StagingTable", async () => {
    const createObjectUrlSpy = vi
      .spyOn(URL, "createObjectURL")
      .mockReturnValue("blob:mock-url");
    const revokeSpy = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => {});
    const clickSpy = vi
      .spyOn(HTMLAnchorElement.prototype, "click")
      .mockImplementation(() => {});

    const results: MatchResult[] = [
      {
        source_track: {
          id: "sp1",
          title: "Track One",
          artists: ["Artist"],
          album: "Album",
          duration_ms: 180_000,
          isrc: null,
          is_explicit: false,
          is_playable: true,
          preview_url: null,
          thumbnail_url: null,
        },
        status: "Exact",
        matched_track: {
          id: "yt1",
          title: "Track One",
          artists: ["Artist"],
          album: "Album",
          duration_ms: 180_000,
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

    await act(async () => {
      root.render(
        <AudioProvider>
          <TransferProvider>
            <StagingTable
              results={results}
              playlistTitle="Export Test"
              sourceService="spotify"
              targetService="ytmusic"
            />
          </TransferProvider>
        </AudioProvider>,
      );
    });

    for (const label of ["Export JSON", "Export CSV", "Export M3U8"]) {
      const btn = container.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
      expect(btn).toBeTruthy();
      await act(async () => {
        btn.click();
      });
    }

    expect(createObjectUrlSpy).toHaveBeenCalledTimes(3);
    expect(clickSpy).toHaveBeenCalledTimes(3);
    createObjectUrlSpy.mockRestore();
    revokeSpy.mockRestore();
    clickSpy.mockRestore();
  });

  it("imports local CSV file in PlaylistSelector and triggers match_custom_tracks", async () => {
    vi.useFakeTimers();
    const matchedRow: MatchResult = {
      source_track: {
        id: "csv-1",
        title: "Imported Song",
        artists: ["Artist"],
        album: "Album",
        duration_ms: 190_000,
        isrc: null,
        is_explicit: false,
        is_playable: true,
        preview_url: null,
        thumbnail_url: null,
      },
      status: "Exact",
      matched_track: {
        id: "yt-csv-1",
        title: "Imported Song",
        artists: ["Artist"],
        album: "Album",
        duration_ms: 190_000,
        isrc: null,
        is_explicit: false,
        is_playable: true,
        preview_url: null,
        thumbnail_url: null,
      },
      candidates: [],
      confidence: 1,
      match_method: "fuzzy",
    };

    mocks.invoke.mockImplementation(async (cmd: string) => {
      if (cmd === "check_auth") return { service: "ytmusic", status: "connected" };
      if (cmd === "check_provider_connection") return null;
      if (cmd === "list_provider_playlists") return [];
      if (cmd === "match_custom_tracks") return [matchedRow];
      return [];
    });

    const onMatchComplete = vi.fn();
    await act(async () => {
      root.render(
        <AuthProvider>
          <PlaylistSelector onMatchComplete={onMatchComplete} />
        </AuthProvider>,
      );
    });

    const fileInput = container.querySelector<HTMLInputElement>(
      'input[aria-label="Import playlist file"]',
    )!;
    const csvFile = {
      name: "chill_mix.csv",
      text: async () => "title,artist\nImported Song,Artist\n",
    } as unknown as File;
    Object.defineProperty(fileInput, "files", {
      value: [csvFile],
      configurable: true,
    });

    await act(async () => {
      fileInput.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await act(async () => {
      vi.runAllTimers();
    });

    expect(mocks.invoke).toHaveBeenCalledWith(
      "match_custom_tracks",
      expect.objectContaining({
        sourceService: "spotify",
        targetService: "ytmusic",
      }),
    );
    expect(onMatchComplete).toHaveBeenCalledTimes(1);
    expect(onMatchComplete.mock.calls[0][1].title).toBe("chill_mix");
    vi.useRealTimers();
  });
});
