// @vitest-environment jsdom
import { act } from "react";
import { createRoot, Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Modal } from "../components/ui/Modal";
import { Sidebar } from "../components/layout/Sidebar";
import { TransferHistory } from "../components/history/TransferHistory";
import { SyncPage } from "../components/sync/SyncPage";
import { SyncReview } from "../components/sync/SyncReview";
import { StagingActionBar } from "../components/diff/StagingActionBar";
import { TransferProvider } from "../context/TransferContext";
import { AudioProvider } from "../context/AudioContext";
import { SyncPlan } from "../types/sync";
import { SourceTrack } from "../types/provider";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));
vi.mock("../utils/tauri", () => ({ isTauri: () => true }));

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

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
  (
    globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  mocks.invoke.mockReset();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe("Phase 2 UI & UX unification", () => {
  it("sets background #app-shell inert and restores focus when Modal closes, blocking Escape when disableDismiss is true", async () => {
    const appShell = document.createElement("div");
    appShell.id = "app-shell";
    const triggerBtn = document.createElement("button");
    triggerBtn.textContent = "Open Dialog";
    appShell.append(triggerBtn);
    document.body.append(appShell);
    triggerBtn.focus();

    const onClose = vi.fn();

    await act(async () => {
      root.render(
        <Modal isOpen onClose={onClose} disableDismiss title="Protected Modal">
          <p>Busy work</p>
        </Modal>,
      );
    });

    expect((appShell as HTMLElement & { inert: boolean }).inert).toBe(true);

    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(onClose).not.toHaveBeenCalled();

    await act(async () => {
      root.render(
        <Modal isOpen={false} onClose={onClose} title="Protected Modal">
          <p>Done</p>
        </Modal>,
      );
    });

    expect((appShell as HTMLElement & { inert: boolean }).inert).toBe(false);
    expect(document.activeElement).toBe(triggerBtn);
    appShell.remove();
  });

  it("shows truthful Local Integrity status and expands recovery actions only for recovery jobs", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "get_transfer_history") {
        return [
          {
            job_id: "healthy-job",
            snapshot_id: "snap-1",
            source_service: "spotify",
            target_service: "ytmusic",
            source_playlist_name: "Clean Mix",
            target_playlist_id: "pl-1",
            is_new_playlist: true,
            total_tracks: 10,
            matched_tracks: 10,
            added_tracks_count: 10,
            pre_existing_count: 0,
            is_rolled_back: false,
            created_at: 1700000000,
            job_state: "completed_verified",
            recovery_required: false,
            last_error: null,
          },
          {
            job_id: "broken-job",
            snapshot_id: "snap-2",
            source_service: "spotify",
            target_service: "ytmusic",
            source_playlist_name: "Uncertain Mix",
            target_playlist_id: "pl-2",
            is_new_playlist: false,
            total_tracks: 5,
            matched_tracks: 5,
            added_tracks_count: 2,
            pre_existing_count: 1,
            is_rolled_back: false,
            created_at: 1700000100,
            job_state: "recovery_required",
            recovery_required: true,
            last_error: "Timeout",
          },
        ];
      }
      if (command === "get_storage_recovery_notice") return null;
      throw new Error(`Unexpected command ${command}`);
    });

    await act(async () => {
      root.render(<TransferHistory />);
    });

    expect(container.textContent).toContain("1 Pending");
    expect(container.textContent).not.toContain("Local IntegrityActive");

    const openInspectButtons = Array.from(container.querySelectorAll("button")).filter(
      (b) => b.textContent?.includes("Inspect recovery") && !b.closest("details"),
    );
    expect(openInspectButtons).toHaveLength(1);
    expect(container.querySelectorAll("details")).toHaveLength(1);
  });

  it("filters Link Playlists selects in SyncPage and supports filter + jump in SyncReview", async () => {
    mocks.invoke.mockImplementation(async (command: string, args?: { service?: string }) => {
      if (command === "list_sync_pairs") return [];
      if (command === "list_provider_playlists") {
        if (args?.service === "spotify") {
          return [
            { id: "sp-1", service: "spotify", title: "Deep Focus", track_count: 12, is_public: false },
            { id: "sp-2", service: "spotify", title: "Gym Beats", track_count: 20, is_public: false },
          ];
        }
        return [
          { id: "yt-1", service: "ytmusic", title: "YT Focus", track_count: 12, is_public: false },
        ];
      }
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

    const filterInput = container.querySelector<HTMLInputElement>(
      'input[placeholder="Filter Spotify playlists..."]',
    )!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(filterInput, "Deep");
      filterInput.dispatchEvent(new Event("input", { bubbles: true }));
    });

    const spSelect = container.querySelector<HTMLSelectElement>(
      'select[aria-label="Spotify playlist"]',
    )!;
    expect(spSelect.options).toHaveLength(2);
    expect(spSelect.options[1].textContent).toBe("Deep Focus");

    const plan: SyncPlan = {
      plan_id: "plan-1",
      pair_id: "pair-1",
      baseline_version: 1,
      pair_stamp: "stamp",
      spotify_before: { complete: true, revision: "r1", occurrences: [] },
      ytmusic_before: { complete: true, revision: "r2", occurrences: [] },
      desired: [
        {
          logical_id: "l1",
          spotify: makeTrack("s1", "Alpha Track"),
          ytmusic: makeTrack("y1", "Alpha Track"),
          spotify_remote_id: null,
          ytmusic_remote_id: null,
        },
        {
          logical_id: "l2",
          spotify: makeTrack("s2", "Beta Track"),
          ytmusic: makeTrack("y2", "Beta Track"),
          spotify_remote_id: null,
          ytmusic_remote_id: null,
        },
        {
          logical_id: "l3",
          spotify: makeTrack("s3", "Gamma Track"),
          ytmusic: makeTrack("y3", "Gamma Track"),
          spotify_remote_id: null,
          ytmusic_remote_id: null,
        },
        {
          logical_id: "l4",
          spotify: makeTrack("s4", "Delta Track"),
          ytmusic: makeTrack("y4", "Delta Track"),
          spotify_remote_id: null,
          ytmusic_remote_id: null,
        },
      ],
      conflicts: [],
      initial: false,
    };

    const onResolve = vi.fn();
    await act(async () => {
      root.render(
        <SyncReview plan={plan} busy={false} onResolve={onResolve} onApply={() => {}} />,
      );
    });

    const jumpInput = container.querySelector<HTMLInputElement>(
      'input[aria-label="Move song 4 to position"]',
    )!;
    const jumpForm = jumpInput.closest("form")!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(jumpInput, "1");
      jumpInput.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      jumpForm.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    });

    const validateBtn = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Validate review"),
    )!;
    await act(async () => {
      validateBtn.click();
    });
    expect(onResolve.mock.calls[0][0].order).toEqual(["l4", "l1", "l2", "l3"]);
  });

  it("renders sidebar attention badges and staging action bar safely", async () => {
    await act(async () => {
      root.render(
        <AudioProvider>
          <TransferProvider>
            <div>
              <Sidebar
                activeTab="playlists"
                onTabChange={() => {}}
                badges={{ transfers: 3, history: 2 }}
              />
              <StagingActionBar
                selectedCount={4}
                totalCount={5}
                exactCount={4}
                amberCount={1}
                redCount={0}
                playingTrackTitle="Starboy"
                onAcceptAllAmbiguous={() => {}}
                onSkipUnresolved={() => {}}
                onDeduplicate={() => {}}
                onStartTransfer={() => {}}
              />
            </div>
          </TransferProvider>
        </AudioProvider>,
      );
    });

    expect(container.textContent).toContain("3");
    expect(container.textContent).toContain("2");
    expect(container.textContent).toContain("4 of 5 selected");
  });
});
