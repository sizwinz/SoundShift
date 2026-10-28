// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { SyncPage } from "../components/sync/SyncPage";
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
const track = {
  id: "a",
  title: "Fixture song",
  artists: ["Fixture artist"],
  duration_ms: 1000,
  is_explicit: false,
  is_playable: true,
};
const occurrence = {
  logical_id: "occ-a",
  spotify: track,
  ytmusic: track,
  spotify_remote_id: null,
  ytmusic_remote_id: "remote-a",
};
const read = {
  complete: true,
  revision: null,
  occurrences: [{ index: 0, track, remote_id: null }],
};
const pair = {
  pair_id: "pair-a",
  spotify: { playlist_id: "sp", title: "Spotify fixture", generation: 1 },
  ytmusic: { playlist_id: "yt", title: "YouTube fixture", generation: 1 },
  baseline: null,
  enabled: false,
  state: "draft",
  next_due_at: null,
  last_run_id: null,
  last_error: null,
};
const plan = {
  plan_id: "plan-a",
  pair_id: "pair-a",
  baseline_version: 0,
  pair_stamp: "stamp",
  spotify_before: read,
  ytmusic_before: read,
  desired: [occurrence],
  conflicts: [],
  initial: true,
};
let cleanup: () => Promise<void>;
afterEach(async () => {
  if (cleanup) await cleanup();
  mocks.invoke.mockReset();
  mocks.listeners.clear();
});
async function mount() {
  (
    globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  cleanup = async () => {
    await act(async () => root.unmount());
    container.remove();
  };
  await act(async () => {
    root.render(<SyncPage />);
  });
  return container;
}
function button(container: HTMLElement, label: string) {
  const found = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.includes(label),
  );
  expect(found).toBeTruthy();
  return found!;
}

it("previews a saved pair and requires explicit review before sending writes", async () => {
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === "list_sync_pairs") return [pair];
    if (command === "preview_sync_pair") return plan;
    throw new Error(`Unexpected ${command}`);
  });
  const container = await mount();
  await act(async () => button(container, "Sync now").click());
  expect(mocks.invoke).toHaveBeenCalledWith("preview_sync_pair", {
    pairId: "pair-a",
    resolution: null,
  });
  expect(container.textContent).toContain("Fixture song");
  expect(button(container, "Apply reviewed sync").disabled).toBe(false);
  expect(
    mocks.invoke.mock.calls.some(
      ([command]) => command === "execute_sync_plan",
    ),
  ).toBe(false);
});

it("applies only the reviewed plan and displays verified completion", async () => {
  mocks.invoke.mockImplementation(
    async (command: string, args: Record<string, unknown>) => {
      if (command === "list_sync_pairs") return [pair];
      if (command === "preview_sync_pair") return plan;
      if (command === "execute_sync_plan")
        return {
          pair_id: pair.pair_id,
          job_id: args.jobId,
          state: "completed_verified",
          event_seq: 4,
          counts: {
            added: 1,
            removed: 0,
            moved: 0,
            failed: 0,
            uncertain: 0,
            unsent: 0,
          },
          verified: true,
          error: null,
        };
      throw new Error(`Unexpected ${command}`);
    },
  );
  const container = await mount();
  await act(async () => button(container, "Sync now").click());
  await act(async () => button(container, "Apply reviewed sync").click());
  expect(mocks.invoke).toHaveBeenCalledWith("execute_sync_plan", {
    planId: "plan-a",
    jobId: expect.stringMatching(/^sync_/),
  });
  expect(container.textContent).toContain("Both playlists verified");
});

it("ignores stale lifecycle events and an outdated command response", async () => {
  let resolveRun!: (value: unknown) => void;
  let jobId = "";
  mocks.invoke.mockImplementation(
    async (command: string, args: Record<string, unknown>) => {
      if (command === "list_sync_pairs") return [pair];
      if (command === "preview_sync_pair") return plan;
      if (command === "execute_sync_plan") {
        jobId = String(args.jobId);
        return new Promise((resolve) => {
          resolveRun = resolve;
        });
      }
      throw new Error(`Unexpected ${command}`);
    },
  );
  const container = await mount();
  await act(async () => button(container, "Sync now").click());
  await act(async () => button(container, "Apply reviewed sync").click());
  await act(async () => {
    mocks.listeners.get("sync:state_changed")?.({
      payload: {
        pair_id: pair.pair_id,
        job_id: jobId,
        state: "recovery_required",
        event_seq: 5,
      },
    });
    mocks.listeners.get("sync:state_changed")?.({
      payload: {
        pair_id: pair.pair_id,
        job_id: jobId,
        state: "completed_verified",
        event_seq: 4,
      },
    });
  });
  await act(async () =>
    resolveRun({
      pair_id: pair.pair_id,
      job_id: jobId,
      state: "running",
      event_seq: 1,
      counts: {
        added: 1,
        removed: 0,
        moved: 0,
        failed: 0,
        uncertain: 1,
        unsent: 0,
      },
      verified: false,
      error: null,
    }),
  );
  expect(container.textContent).toContain("recovery required");
  expect(container.textContent).not.toContain("Both playlists verified");
});

it("surfaces rejected controls without pretending the run paused", async () => {
  let jobId = "";
  mocks.invoke.mockImplementation(
    async (command: string, args: Record<string, unknown>) => {
      if (command === "list_sync_pairs") return [pair];
      if (command === "preview_sync_pair") return plan;
      if (command === "execute_sync_plan") {
        jobId = String(args.jobId);
        return new Promise(() => {});
      }
      if (command === "control_sync_run")
        throw new Error("Control checkpoint unavailable");
      throw new Error(`Unexpected ${command}`);
    },
  );
  const container = await mount();
  await act(async () => button(container, "Sync now").click());
  await act(async () => button(container, "Apply reviewed sync").click());
  await act(async () =>
    mocks.listeners.get("sync:state_changed")?.({
      payload: {
        pair_id: pair.pair_id,
        job_id: jobId,
        state: "running",
        event_seq: 1,
      },
    }),
  );
  await act(async () => button(container, "Pause sync").click());
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "Control checkpoint unavailable",
  );
  expect(button(container, "Pause sync").disabled).toBe(false);
});

it("links selected existing playlists and opens an initial review without writing", async () => {
  let created = false;
  mocks.invoke.mockImplementation(
    async (command: string, args: Record<string, unknown>) => {
      if (command === "list_sync_pairs") return created ? [pair] : [];
      if (command === "list_provider_playlists")
        return [
          {
            id: args.service === "spotify" ? "sp" : "yt",
            title:
              args.service === "spotify"
                ? "Spotify fixture"
                : "YouTube fixture",
            service: args.service,
            track_count: 1,
            is_public: false,
          },
        ];
      if (command === "create_sync_pair") {
        created = true;
        return pair;
      }
      if (command === "preview_sync_pair") return plan;
      throw new Error(`Unexpected ${command}`);
    },
  );
  const container = await mount();
  await act(async () => button(container, "Link playlists").click());
  for (const [label, value] of [
    ["Spotify playlist", "sp"],
    ["YouTube Music playlist", "yt"],
  ]) {
    await act(async () => {
      const select = container.querySelector(
        `select[aria-label="${label}"]`,
      ) as HTMLSelectElement;
      expect(select).toBeTruthy();
      select.value = value;
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
  }
  await act(async () => button(container, "Review linked pair").click());
  expect(mocks.invoke).toHaveBeenCalledWith("create_sync_pair", {
    spotifyId: "sp",
    spotifyTitle: "Spotify fixture",
    ytmusicId: "yt",
    ytmusicTitle: "YouTube fixture",
  });
  expect(container.textContent).toContain("Review initial merge");
  expect(
    mocks.invoke.mock.calls.some(
      ([command]) => command === "execute_sync_plan",
    ),
  ).toBe(false);
});

it("enables the hourly schedule only for an initialized verified pair", async () => {
  const initialized = {
    ...pair,
    state: "synced",
    baseline: {
      version: 1,
      occurrences: [occurrence],
      spotify: read,
      ytmusic: read,
    },
  };
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === "list_sync_pairs") return [initialized];
    if (command === "set_sync_schedule")
      return { ...initialized, enabled: true, next_due_at: 3600 };
    throw new Error(`Unexpected ${command}`);
  });
  const container = await mount();
  await act(async () => button(container, "Enable hourly sync").click());
  expect(mocks.invoke).toHaveBeenCalledWith("set_sync_schedule", {
    pairId: pair.pair_id,
    enabled: true,
  });
});

it("requires backend revalidation after choosing an ambiguous match", async () => {
  const choice = { ...track, id: "chosen", title: "Chosen recording" };
  const unresolved = {
    ...plan,
    desired: [{ ...occurrence, ytmusic: null }],
    conflicts: [
      {
        code: "match_required",
        logical_id: "occ-a",
        detail: "Choose a match",
        candidates: [choice],
      },
    ],
  };
  mocks.invoke.mockImplementation(
    async (command: string, args: Record<string, unknown>) => {
      if (command === "list_sync_pairs") return [pair];
      if (command === "preview_sync_pair")
        return args.resolution ? { ...plan, plan_id: "resolved" } : unresolved;
      throw new Error(`Unexpected ${command}`);
    },
  );
  const container = await mount();
  await act(async () => button(container, "Sync now").click());
  await act(async () => button(container, "Use Chosen recording").click());
  expect(button(container, "Apply reviewed sync").disabled).toBe(true);
  await act(async () => button(container, "Validate review").click());
  expect(mocks.invoke).toHaveBeenLastCalledWith("preview_sync_pair", {
    pairId: pair.pair_id,
    resolution: {
      plan_id: plan.plan_id,
      order: ["occ-a"],
      choices: [{ logical_id: "occ-a", side: "ytmusic", track: choice }],
      strategy: null,
    },
  });
  expect(button(container, "Apply reviewed sync").disabled).toBe(false);
});

it("inspects an interrupted run and blocks replay of uncertain operations", async () => {
  const summary = {
    pair_id: pair.pair_id,
    job_id: "sync-old",
    state: "recovery_required",
    event_seq: 5,
    counts: {
      added: 1,
      removed: 0,
      moved: 0,
      failed: 0,
      uncertain: 1,
      unsent: 2,
    },
    verified: false,
    error: null,
  };
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === "list_sync_pairs")
      return [
        { ...pair, state: "recovery_required", last_run_id: summary.job_id },
      ];
    if (command === "inspect_sync_run")
      return {
        summary,
        operations: [
          {
            operation_id: "op-one",
            side: "spotify",
            kind: "add",
            status: "uncertain",
            attempt_count: 1,
            detail: "Response lost",
          },
        ],
        can_resume: false,
        next_action: "Uncertain operations cannot be replayed",
      };
    throw new Error(`Unexpected ${command}`);
  });
  const container = await mount();
  await act(async () => button(container, "Inspect sync").click());
  expect(mocks.invoke).toHaveBeenCalledWith("inspect_sync_run", {
    jobId: "sync-old",
  });
  expect(container.textContent).toContain(
    "Uncertain operations cannot be replayed",
  );
  expect(button(container, "Resume recorded sync").disabled).toBe(true);
});

it("unlinks only after confirmation and never deletes either provider playlist", async () => {
  let unlinked = false;
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === "list_sync_pairs") return unlinked ? [] : [pair];
    if (command === "unlink_sync_pair") {
      unlinked = true;
      return;
    }
    throw new Error(`Unexpected ${command}`);
  });
  const container = await mount();
  await act(async () => button(container, "Unlink pair").click());
  expect(
    mocks.invoke.mock.calls.some(([command]) => command === "unlink_sync_pair"),
  ).toBe(false);
  expect(document.body.textContent).toContain(
    "Both playlists and recorded run history are preserved",
  );
  await act(async () => button(document.body, "Confirm unlink").click());
  expect(mocks.invoke).toHaveBeenCalledWith("unlink_sync_pair", {
    pairId: pair.pair_id,
  });
});

it("refreshes disabled pair state when account credentials change", async () => {
  let changed = false;
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === "list_sync_pairs")
      return [
        {
          ...pair,
          state: changed ? "authentication_required" : "synced",
          enabled: !changed,
          baseline: {
            version: 1,
            occurrences: [],
            spotify: read,
            ytmusic: read,
          },
        },
      ];
    throw new Error(`Unexpected ${command}`);
  });
  const container = await mount();
  expect(container.textContent).toContain("Disable hourly sync");
  changed = true;
  await act(async () =>
    mocks.listeners.get("auth:status_changed")?.({
      payload: { service: "spotify", status: "disconnected" },
    }),
  );
  expect(container.textContent).toContain("authentication required");
  expect(container.textContent).not.toContain("Disable hourly sync");
});

it("does not leave a failed start looking like active work", async () => {
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === "list_sync_pairs") return [pair];
    if (command === "preview_sync_pair") return plan;
    if (command === "execute_sync_plan")
      throw new Error("Preview stale; inspect before retrying");
    throw new Error(`Unexpected ${command}`);
  });
  const container = await mount();
  await act(async () => button(container, "Sync now").click());
  await act(async () => button(container, "Apply reviewed sync").click());
  expect(container.querySelector('[role="alert"]')?.textContent).toContain(
    "Preview stale",
  );
  expect(container.textContent).toContain("recovery required");
  expect(container.textContent).not.toContain("starting");
});
