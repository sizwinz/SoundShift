import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { RefreshCw } from "lucide-react";
import { Button, Card, Modal } from "../ui";
import { isTauri } from "../../utils/tauri";
import {
  SyncPair,
  SyncPlan,
  SyncRunSummary,
  SyncUpdate,
  SyncResolution,
  SyncInspection,
} from "../../types/sync";
import { Playlist } from "../../types/provider";
import { SyncReview } from "./SyncReview";

const terminal = (state: string) =>
  [
    "completed_verified",
    "cancelled",
    "recovery_required",
    "manual_review",
  ].includes(state);
const emptyCounts = {
  added: 0,
  removed: 0,
  moved: 0,
  failed: 0,
  uncertain: 0,
  unsent: 0,
};

export function SyncPage() {
  const [pairs, setPairs] = useState<SyncPair[]>([]);
  const [plan, setPlan] = useState<SyncPlan | null>(null);
  const [run, setRun] = useState<SyncRunSummary | null>(null);
  const [inspection, setInspection] = useState<SyncInspection | null>(null);
  const [unlinking, setUnlinking] = useState<SyncPair | null>(null);
  const [busy, setBusy] = useState(false);
  const [controlPending, setControlPending] = useState(false);
  const [linking, setLinking] = useState(false);
  const [catalog, setCatalog] = useState<[Playlist[], Playlist[]]>([[], []]);
  const [spotifyId, setSpotifyId] = useState("");
  const [youtubeId, setYoutubeId] = useState("");
  const [error, setError] = useState<string | null>(null);
  const request = useRef(0);
  const active = useRef<SyncUpdate | null>(null);
  const listenersReady = useRef<Promise<void> | null>(null);
  const refresh = useCallback(async () => {
    if (isTauri()) setPairs(await invoke<SyncPair[]>("list_sync_pairs"));
  }, []);
  useEffect(() => {
    void refresh().catch((e) => setError(String(e)));
  }, [refresh]);
  const acceptUpdate = useCallback((update: SyncUpdate) => {
    const previous = active.current;
    if (
      !previous ||
      update.job_id !== previous.job_id ||
      update.pair_id !== previous.pair_id ||
      update.event_seq <= previous.event_seq ||
      terminal(previous.state)
    )
      return;
    active.current = update;
    setRun((current) => ({
      ...update,
      counts: current?.counts ?? emptyCounts,
      verified: false,
      error: current?.error ?? null,
    }));
  }, []);
  const freezeRun = useCallback(() => {
    const previous = active.current;
    if (!previous || terminal(previous.state)) return;
    active.current = { ...previous, state: "recovery_required" };
    setRun((current) =>
      current
        ? { ...current, state: "recovery_required", verified: false }
        : current,
    );
  }, []);
  const acceptSummary = useCallback(
    (summary: SyncRunSummary) => {
      const previous = active.current;
      if (
        previous &&
        summary.job_id === previous.job_id &&
        summary.pair_id === previous.pair_id &&
        summary.error &&
        !terminal(summary.state) &&
        !terminal(previous.state)
      ) {
        freezeRun();
        return;
      }
      if (
        !previous ||
        summary.job_id !== previous.job_id ||
        summary.pair_id !== previous.pair_id ||
        summary.event_seq < previous.event_seq ||
        (terminal(previous.state) && previous.state !== summary.state)
      )
        return;
      active.current = summary;
      setRun(summary);
    },
    [freezeRun],
  );
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    const cleanup: (() => void)[] = [];
    listenersReady.current = Promise.all([
      listen<SyncUpdate>("sync:state_changed", (event) => {
        if (!disposed) acceptUpdate(event.payload);
      }),
      listen("sync:pairs_changed", () => {
        if (!disposed) void refresh().catch((e) => setError(String(e)));
      }),
      listen("auth:status_changed", () => {
        if (!disposed) void refresh().catch((e) => setError(String(e)));
      }),
    ]).then((unlisteners) => {
      if (disposed) unlisteners.forEach((stop) => stop());
      else cleanup.push(...unlisteners);
    });
    return () => {
      disposed = true;
      cleanup.forEach((stop) => stop());
    };
  }, [acceptUpdate, refresh]);
  const preview = async (
    pairId: string,
    resolution: SyncResolution | null = null,
  ) => {
    const token = ++request.current;
    setBusy(true);
    setError(null);
    setPlan(null);
    try {
      const next = await invoke<SyncPlan>("preview_sync_pair", {
        pairId,
        resolution,
      });
      if (token === request.current) setPlan(next);
    } catch (e) {
      setError(String(e));
    } finally {
      if (token === request.current) setBusy(false);
    }
  };
  const apply = async () => {
    if (!plan || busy) return;
    const jobId = `sync_${crypto.randomUUID()}`;
    setBusy(true);
    setError(null);
    const initial = {
      pair_id: plan.pair_id,
      job_id: jobId,
      state: "starting",
      event_seq: 0,
    };
    active.current = initial;
    setRun({ ...initial, counts: emptyCounts, verified: false, error: null });
    try {
      await listenersReady.current;
      const summary = await invoke<SyncRunSummary>("execute_sync_plan", {
        planId: plan.plan_id,
        jobId,
      });
      acceptSummary(summary);
      setPlan(null);
      if (summary.error) setError(summary.error);
      await refresh();
    } catch (e) {
      setError(String(e));
      setPlan(null);
      freezeRun();
      void refresh().catch(() => {});
    } finally {
      setBusy(false);
    }
  };
  const requestControl = async (action: "pause" | "resume" | "cancel") => {
    const current = active.current;
    if (!current || controlPending) return;
    setControlPending(true);
    setError(null);
    try {
      const update = await invoke<SyncUpdate>("control_sync_run", {
        jobId: current.job_id,
        action,
      });
      acceptUpdate(update);
    } catch (e) {
      setError(String(e));
    } finally {
      setControlPending(false);
    }
  };
  const openLink = async () => {
    setLinking(true);
    setBusy(true);
    setError(null);
    try {
      const playlists = await Promise.all([
        invoke<Playlist[]>("list_provider_playlists", {
          service: "spotify",
          refresh: true,
        }),
        invoke<Playlist[]>("list_provider_playlists", {
          service: "ytmusic",
          refresh: true,
        }),
      ]);
      setCatalog(playlists);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const createPair = async () => {
    const spotify = catalog[0].find((p) => p.id === spotifyId);
    const youtube = catalog[1].find((p) => p.id === youtubeId);
    if (!spotify || !youtube || busy) return;
    setBusy(true);
    setError(null);
    try {
      const pair = await invoke<SyncPair>("create_sync_pair", {
        spotifyId: spotify.id,
        spotifyTitle: spotify.title,
        ytmusicId: youtube.id,
        ytmusicTitle: youtube.title,
      });
      setLinking(false);
      await refresh();
      await preview(pair.pair_id);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const toggleSchedule = async (pair: SyncPair) => {
    setBusy(true);
    setError(null);
    try {
      const updated = await invoke<SyncPair>("set_sync_schedule", {
        pairId: pair.pair_id,
        enabled: !pair.enabled,
      });
      setPairs((current) =>
        current.map((p) => (p.pair_id === updated.pair_id ? updated : p)),
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const inspectRun = async (jobId: string, reconcile = false) => {
    const token = ++request.current;
    setBusy(true);
    setError(null);
    try {
      await listenersReady.current;
      const next = await invoke<SyncInspection>(
        reconcile ? "reconcile_sync_run" : "inspect_sync_run",
        { jobId },
      );
      if (token !== request.current) return;
      const previous = active.current;
      if (
        previous?.job_id === jobId &&
        (next.summary.event_seq < previous.event_seq ||
          (previous.state === "completed_verified" &&
            next.summary.state !== previous.state))
      )
        return;
      active.current = next.summary;
      setRun(next.summary);
      setInspection(next);
    } catch (e) {
      setError(String(e));
    } finally {
      if (token === request.current) setBusy(false);
    }
  };
  const resumeRecorded = async () => {
    if (!inspection?.can_resume || busy) return;
    setBusy(true);
    setError(null);
    const jobId = inspection.summary.job_id;
    active.current = { ...inspection.summary, state: "starting" };
    setRun({ ...inspection.summary, state: "starting", verified: false });
    try {
      await listenersReady.current;
      const summary = await invoke<SyncRunSummary>("resume_sync_run", {
        jobId,
      });
      acceptSummary(summary);
      setInspection(null);
      await refresh();
    } catch (e) {
      setError(String(e));
      if (active.current?.job_id === jobId) {
        active.current = { ...active.current, state: "recovery_required" };
        setRun((current) =>
          current
            ? { ...current, state: "recovery_required", verified: false }
            : current,
        );
      }
    } finally {
      setBusy(false);
    }
  };
  const unlink = async () => {
    if (!unlinking || busy) return;
    setBusy(true);
    setError(null);
    try {
      await invoke("unlink_sync_pair", { pairId: unlinking.pair_id });
      if (plan?.pair_id === unlinking.pair_id) setPlan(null);
      setUnlinking(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const revalidate = async (pairId: string) => {
    setBusy(true);
    setError(null);
    try {
      await invoke("revalidate_sync_pair", { pairId });
      await refresh();
      await preview(pairId);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <section className="space-y-5" aria-label="Playlist sync">
      <header>
        <div className="flex items-center gap-2">
          <RefreshCw className="w-5 h-5 text-emerald-400" />
          <h1 className="text-xl font-semibold">Playlist sync</h1>
        </div>
        <p className="mt-2 text-sm text-zinc-400 max-w-2xl">
          Keep songs and order aligned between Spotify and YouTube Music. Hourly
          sync runs while SoundShift is open.
        </p>
      </header>
      {error && (
        <div
          role="alert"
          className="p-3 border border-red-900 rounded-lg text-sm text-red-300"
        >
          {error}
        </div>
      )}
      <Button disabled={busy || !isTauri()} onClick={() => void openLink()}>
        Link playlists
      </Button>
      {linking && (
        <Card className="p-4 space-y-4">
          <h2 className="text-sm font-medium">
            Choose existing editable playlists
          </h2>
          <p className="text-xs text-zinc-400">
            The first merge requires review. Names, descriptions, artwork, and
            visibility stay local.
          </p>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
            {([0, 1] as const).map((side) => (
              <label key={side} className="space-y-2 text-sm">
                <span>{side === 0 ? "Spotify" : "YouTube Music"}</span>
                <select
                  aria-label={
                    side === 0 ? "Spotify playlist" : "YouTube Music playlist"
                  }
                  className="block w-full bg-zinc-950 border border-zinc-700 rounded-md p-2 text-sm"
                  value={side === 0 ? spotifyId : youtubeId}
                  onChange={(e) =>
                    side === 0
                      ? setSpotifyId(e.target.value)
                      : setYoutubeId(e.target.value)
                  }
                  disabled={busy}
                >
                  <option value="">Choose a playlist</option>
                  {catalog[side].map((p) => (
                    <option value={p.id} key={p.id}>
                      {p.title}
                    </option>
                  ))}
                </select>
              </label>
            ))}
          </div>
          <div className="flex gap-2">
            <Button
              disabled={busy || !spotifyId || !youtubeId}
              onClick={() => void createPair()}
            >
              Review linked pair
            </Button>
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() => setLinking(false)}
            >
              Close
            </Button>
          </div>
        </Card>
      )}
      {!isTauri() && (
        <p className="text-sm text-zinc-400">
          Open the desktop app to connect and sync playlists.
        </p>
      )}
      {run && (
        <Card className="p-4 space-y-3" aria-live="polite">
          <h2 className="text-sm font-medium">
            {run.verified
              ? "Both playlists verified"
              : run.state.replaceAll("_", " ")}
          </h2>
          <p className="text-xs text-zinc-400">
            {run.counts.added} added · {run.counts.removed} removed ·{" "}
            {run.counts.moved} moved · {run.counts.uncertain} uncertain
          </p>
          {!terminal(run.state) && (
            <div className="flex gap-2 flex-wrap">
              <Button
                size="sm"
                disabled={controlPending || run.state !== "running"}
                onClick={() => void requestControl("pause")}
              >
                Pause sync
              </Button>
              <Button
                size="sm"
                disabled={
                  controlPending || !["pausing", "paused"].includes(run.state)
                }
                onClick={() => void requestControl("resume")}
              >
                Resume sync
              </Button>
              <Button
                size="sm"
                disabled={
                  controlPending ||
                  ["starting", "cancelling"].includes(run.state)
                }
                onClick={() => void requestControl("cancel")}
              >
                Cancel sync
              </Button>
              {controlPending && (
                <span className="text-xs text-zinc-400 self-center">
                  Waiting for acknowledgment…
                </span>
              )}
            </div>
          )}
        </Card>
      )}
      {pairs.map((pair) => (
        <Card key={pair.pair_id} className="p-4 space-y-3">
          <div className="flex items-center justify-between gap-4 flex-wrap">
            <div>
              <h2 className="font-medium text-sm">
                {pair.spotify.title} ↔ {pair.ytmusic.title}
              </h2>
              <p className="text-xs text-zinc-400 mt-1">
                {pair.state.replaceAll("_", " ")}
              </p>
            </div>
            <div className="flex gap-2 flex-wrap">
              <Button
                size="sm"
                disabled={
                  busy ||
                  [
                    "running",
                    "paused",
                    "pausing",
                    "cancelling",
                    "checking",
                  ].includes(pair.state)
                }
                onClick={() => void preview(pair.pair_id)}
              >
                Sync now
              </Button>
              <Button
                size="sm"
                variant="secondary"
                aria-pressed={pair.enabled}
                disabled={
                  busy ||
                  (!pair.enabled && (!pair.baseline || pair.state !== "synced"))
                }
                onClick={() => void toggleSchedule(pair)}
              >
                {pair.enabled ? "Disable hourly sync" : "Enable hourly sync"}
              </Button>
              {pair.last_run_id && (
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={busy}
                  onClick={() => void inspectRun(pair.last_run_id!)}
                >
                  Inspect sync
                </Button>
              )}
              {pair.state === "authentication_required" && (
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={busy}
                  onClick={() => void revalidate(pair.pair_id)}
                >
                  Revalidate account binding
                </Button>
              )}
              <Button
                size="sm"
                variant="ghost"
                disabled={
                  busy ||
                  [
                    "running",
                    "paused",
                    "pausing",
                    "cancelling",
                    "checking",
                  ].includes(pair.state)
                }
                onClick={() => setUnlinking(pair)}
              >
                Unlink pair
              </Button>
            </div>
          </div>
          <p className="text-xs text-zinc-500">
            {pair.enabled && pair.next_due_at
              ? `Next check: ${new Date(pair.next_due_at * 1000).toLocaleString()}`
              : "Hourly schedule is off"}
          </p>
          {pair.last_error && (
            <p className="text-sm text-amber-300">{pair.last_error}</p>
          )}
        </Card>
      ))}
      {inspection && (
        <Card className="p-4 space-y-3">
          <h2 className="text-sm font-medium">Recorded sync inspection</h2>
          <p className="text-sm text-zinc-400">{inspection.next_action}</p>
          <p className="text-xs text-zinc-500">
            {inspection.summary.counts.failed} failed ·{" "}
            {inspection.summary.counts.uncertain} uncertain ·{" "}
            {inspection.summary.counts.unsent} unsent
          </p>
          <ul className="space-y-2 max-h-60 overflow-y-auto">
            {inspection.operations.map((operation) => (
              <li
                key={operation.operation_id}
                className="text-xs text-zinc-400"
              >
                {operation.side} · {operation.kind} · {operation.status} ·{" "}
                {operation.attempt_count} attempts
                {operation.detail && (
                  <p className="mt-1 text-amber-300">{operation.detail}</p>
                )}
              </li>
            ))}
          </ul>
          <div className="flex gap-2 flex-wrap">
            <Button
              size="sm"
              variant="secondary"
              disabled={busy}
              onClick={() => void inspectRun(inspection.summary.job_id, true)}
            >
              Reconcile remote state
            </Button>
            <Button
              size="sm"
              disabled={
                busy ||
                !inspection.can_resume ||
                !["cancelled", "recovery_required", "manual_review"].includes(
                  inspection.summary.state,
                )
              }
              onClick={() => void resumeRecorded()}
            >
              Resume recorded sync
            </Button>
          </div>
        </Card>
      )}
      {pairs.length === 0 && (
        <Card className="p-6 text-sm text-zinc-400">
          Link two existing playlists to review their initial merge.
        </Card>
      )}
      {plan && (
        <SyncReview
          key={plan.plan_id}
          plan={plan}
          busy={busy}
          onResolve={(resolution) => void preview(plan.pair_id, resolution)}
          onApply={() => void apply()}
        />
      )}
      <Modal
        isOpen={unlinking !== null}
        onClose={() => {
          if (!busy) setUnlinking(null);
        }}
        title="Unlink playlist pair"
        description="Both playlists and recorded run history are preserved."
      >
        <div className="space-y-4">
          <p className="text-sm text-zinc-400">
            Hourly checks stop for this pair. Unlinking does not resolve
            uncertain recorded writes.
          </p>
          <div className="flex gap-2">
            <Button disabled={busy} onClick={() => void unlink()}>
              Confirm unlink
            </Button>
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() => setUnlinking(null)}
            >
              Keep linked
            </Button>
          </div>
        </div>
      </Modal>
    </section>
  );
}
