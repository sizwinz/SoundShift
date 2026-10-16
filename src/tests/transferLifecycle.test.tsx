// @vitest-environment jsdom
import { act } from "react";
import { createRoot, Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TransferProvider, useTransfer } from "../context/TransferContext";
import { TelemetryDrawer } from "../components/transfer/TelemetryDrawer";
import { BatchTransferConfig, BatchTransferSummary, LifecycleUpdate } from "../types/transfer";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  handlers: new Map<string, (event: { payload: unknown }) => void>(),
  unlisten: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("../utils/tauri", () => ({ isTauri: () => true }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (name: string, handler: (event: { payload: unknown }) => void) => {
    mocks.handlers.set(name, handler);
    return () => { mocks.handlers.delete(name); mocks.unlisten(name); };
  },
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

let current: ReturnType<typeof useTransfer>;
let root: Root;
let container: HTMLDivElement;
const transfers = new Map<string, ReturnType<typeof deferred<BatchTransferSummary>>>();
let reply = deferred<LifecycleUpdate>();

function Probe() {
  current = useTransfer();
  return <TelemetryDrawer />;
}

const config = (id = "job_one"): BatchTransferConfig => ({
  job_id: id, source_service: "spotify", target_service: "ytmusic", source_playlist_name: "test",
  is_new_playlist: true, tracks: [], concurrency: 1,
});
async function start(id = "job_one") {
  await act(async () => { void current.startTransfer(config(id)); });
}
async function state(state: LifecycleUpdate["state"], event_seq: number, job_id = "job_one") {
  await emit("job:state_changed", { job_id, state, event_seq });
}
async function emit(name: string, payload: unknown) {
  await act(async () => { mocks.handlers.get(name)!({ payload }); });
}
const button = (title: string) => container.querySelector<HTMLButtonElement>(`button[title="${title}"]`)!;

beforeEach(async () => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  mocks.handlers.clear(); mocks.invoke.mockReset(); mocks.unlisten.mockReset(); transfers.clear();
  reply = deferred<LifecycleUpdate>();
  mocks.invoke.mockImplementation((command: string, args: { config?: BatchTransferConfig }) => {
    if (command === "start_batch_transfer") {
      const pending = deferred<BatchTransferSummary>();
      transfers.set(args.config!.job_id, pending);
      return pending.promise;
    }
    return reply.promise;
  });
  container = document.createElement("div"); document.body.append(container);
  root = createRoot(container);
  await act(async () => { root.render(<TransferProvider><Probe /></TransferProvider>); });
});
afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe("real transfer lifecycle event handlers and controls", () => {
  it("does not infer lifecycle state from progress events", async () => {
    await start();
    expect(button("Pause Transfer").disabled).toBe(true);
    await state("running", 1);
    await state("pausing", 2);
    await emit("transfer:progress", { job_id: "job_one", stage: "completed", processed: 2, total: 2, successful: 2 });
    expect(current.stage).toBe("pausing");
    expect(container.textContent).toContain("Waiting for in-flight requests and checkpoints");
    expect(button("Pause Transfer").disabled).toBe(true);
    await state("paused", 3);
    expect(button("Resume Transfer").disabled).toBe(false);
  });

  it("ignores stale sequences, other jobs, and late terminal regressions", async () => {
    await start(); await state("running", 1); await state("paused", 3);
    await state("running", 2); await state("cancelled", 99, "unrelated_job");
    expect(current.stage).toBe("paused");
    await state("completed_verified", 4);
    await state("running", 100);
    expect(current.stage).toBe("completed");
  });

  it("a delayed control reply cannot undo a newer acknowledgment", async () => {
    await start(); await state("running", 1);
    let request!: Promise<void>;
    await act(async () => { request = current.pauseTransfer(); });
    expect(current.controlPending).toBe(true);
    expect(button("Pause Transfer").disabled).toBe(true);
    await state("pausing", 2); await state("paused", 3);
    await act(async () => { reply.resolve({ job_id: "job_one", state: "pausing", event_seq: 2 }); await request; });
    expect(current.stage).toBe("paused");
    expect(current.controlPending).toBe(false);
  });

  it("cancel remains pending until backend acknowledgment", async () => {
    await start(); await state("running", 1);
    let request!: Promise<void>;
    await act(async () => { request = current.cancelTransfer(); });
    await act(async () => { reply.resolve({ job_id: "job_one", state: "cancelling", event_seq: 2 }); await request; });
    expect(current.stage).toBe("cancelling");
    expect(button("Cancel Transfer").disabled).toBe(true);
    await state("cancelled", 3);
    expect(current.stage).toBe("cancelled");
    expect(container.querySelector('button[title="Cancel Transfer"]')).toBeNull();
  });

  it("shows control errors without pretending pause succeeded", async () => {
    await start(); await state("running", 1);
    let request!: Promise<void>;
    await act(async () => { request = current.pauseTransfer(); });
    await act(async () => { reply.reject("Storage temporarily unavailable"); await request; });
    expect(current.stage).toBe("transferring");
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Storage temporarily unavailable");
    expect(current.controlPending).toBe(false);
  });

  it("deduplicates concurrent control requests before React rerenders", async () => {
    await start(); await state("running", 1);
    let first!: Promise<void>; let second!: Promise<void>;
    await act(async () => { first = current.pauseTransfer(); second = current.pauseTransfer(); });
    expect(mocks.invoke.mock.calls.filter(([name]) => name === "pause_batch_transfer")).toHaveLength(1);
    await act(async () => { reply.resolve({ job_id: "job_one", state: "paused", event_seq: 3 }); await Promise.all([first, second]); });
    expect(current.stage).toBe("paused");
  });

  it("does not let an older transfer response replace the current job", async () => {
    await start(); await state("running", 1);
    await start("job_two"); await state("running", 1, "job_two");
    await act(async () => {
      transfers.get("job_one")!.resolve({ job_id: "job_one", snapshot_id: "old", target_playlist_id: "target", total_tracks: 1, successful_tracks: 1,
        failed_tracks: 0, is_cancelled: false, state: "completed_verified", event_seq: 9, audit: { is_verified: true, total_expected: 1, total_found: 1, missing_ids: [] } });
    });
    expect(current.stage).toBe("transferring");
    expect(current.activeConfig?.job_id).toBe("job_two");
    expect(current.summary).toBeNull();
  });

  it("uncertain invoke failures remain recovery-required despite delayed events", async () => {
    await start(); await state("running", 1);
    await act(async () => { transfers.get("job_one")!.reject("Remote outcome requires review"); });
    expect(current.stage).toBe("recovery_required");
    await state("running", 2);
    expect(current.stage).toBe("recovery_required");
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Recovery required");
  });

  it("unsubscribes all native events when the provider unmounts", async () => {
    expect(mocks.handlers.has("job:state_changed")).toBe(true);
    await act(async () => root.unmount());
    expect(mocks.handlers.size).toBe(0);
    expect(mocks.unlisten).toHaveBeenCalledWith("job:state_changed");
    root = createRoot(container);
  });
});
