import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  RefreshCw,
  Link2,
  Clock,
  ShieldCheck,
  AlertTriangle,
  ArrowLeftRight,
  Pause,
  Play,
  XSquare,
  Unlink,
  Search,
} from "lucide-react";
import { Badge, BadgeVariant, Button, Card, Modal, SearchInput } from "../ui";
import { ProviderLogo } from "../providers/ProviderLogo";
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

interface SyncProgressEvent {
  pair_id: string;
  job_id: string;
  processed: number;
  total: number;
  side?: string;
  track_title?: string;
}

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

function syncStateBadgeVariant(state: string, verified?: boolean): BadgeVariant {
  if (verified || state === "synced" || state === "completed_verified") return "exact";
  if (["recovery_required", "manual_review", "authentication_required"].includes(state)) {
    return state === "recovery_required" || state === "manual_review" ? "unmatched" : "amber";
  }
  if (["running", "starting", "checking"].includes(state)) return "connected";
  if (["paused", "pausing", "cancelling", "cancelled"].includes(state)) return "amber";
  return "neutral";
}

function formatDueCountdown(nextDueAt: number | null): string {
  if (!nextDueAt) return "Hourly schedule is off";
  const nowSec = Math.floor(Date.now() / 1000);
  const diffSec = nextDueAt - nowSec;
  const formatted = new Date(nextDueAt * 1000).toLocaleString();
  if (diffSec <= 0) return `Next check: due now (${formatted})`;
  const mins = Math.ceil(diffSec / 60);
  if (mins < 60) return `Next check: in ${mins}m (${formatted})`;
  const hrs = Math.floor(mins / 60);
  const rem = mins % 60;
  return `Next check: in ${hrs}h ${rem}m (${formatted})`;
}

export function SyncPage() {
  const [pairs, setPairs] = useState<SyncPair[]>([]);
  const [plan, setPlan] = useState<SyncPlan | null>(null);
  const [run, setRun] = useState<SyncRunSummary | null>(null);
  const [syncProgress, setSyncProgress] = useState<SyncProgressEvent | null>(null);
  const [inspection, setInspection] = useState<SyncInspection | null>(null);
  const [unlinking, setUnlinking] = useState<SyncPair | null>(null);
  const [busy, setBusy] = useState(false);
  const [controlPending, setControlPending] = useState(false);
  const [linking, setLinking] = useState(false);
  const [catalog, setCatalog] = useState<[Playlist[], Playlist[]]>([[], []]);
  const [spotifyId, setSpotifyId] = useState("");
  const [youtubeId, setYoutubeId] = useState("");
  const [spotifyFilter, setSpotifyFilter] = useState("");
  const [youtubeFilter, setYoutubeFilter] = useState("");
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
    if (terminal(update.state)) {
      setSyncProgress(null);
    }
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
    setSyncProgress(null);
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
      setSyncProgress(null);
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
      listen<SyncProgressEvent>("sync:progress", (event) => {
        if (!disposed && active.current && event.payload.job_id === active.current.job_id) {
          setSyncProgress(event.payload);
        }
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
    setSyncProgress(null);
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
    setSpotifyFilter("");
    setYoutubeFilter("");
    try {
      const playlists = await Promise.all([
        invoke<Playlist[]>("list_provider_playlists", {
          service: "spotify",
          refresh: false,
        }),
        invoke<Playlist[]>("list_provider_playlists", {
          service: "ytmusic",
          refresh: false,
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

  const filteredCatalog = useMemo(() => {
    const spQuery = spotifyFilter.trim().toLowerCase();
    const ytQuery = youtubeFilter.trim().toLowerCase();
    return [
      spQuery
        ? catalog[0].filter((p) => p.id === spotifyId || p.title.toLowerCase().includes(spQuery))
        : catalog[0],
      ytQuery
        ? catalog[1].filter((p) => p.id === youtubeId || p.title.toLowerCase().includes(ytQuery))
        : catalog[1],
    ] as [Playlist[], Playlist[]];
  }, [catalog, spotifyFilter, youtubeFilter, spotifyId, youtubeId]);

  const progressPct =
    syncProgress && syncProgress.total > 0
      ? Math.min(100, Math.round((syncProgress.processed / syncProgress.total) * 100))
      : null;

  return (
    <section className="flex flex-col gap-6 max-w-[1440px] mx-auto w-full pb-16" aria-label="Playlist sync">
      {/* Header Bar */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 pb-3 border-b border-white/[0.08]">
        <div>
          <h1 className="text-base font-semibold text-zinc-100 flex items-center gap-2">
            <RefreshCw className="w-4 h-4 text-emerald-400" />
            <span>Playlist sync</span>
          </h1>
          <p className="text-xs text-zinc-400 mt-0.5 max-w-2xl">
            Keep songs and order aligned between Spotify and YouTube Music. Hourly sync runs while SoundShift is open.
          </p>
        </div>

        <Button
          variant="primary"
          size="sm"
          disabled={busy || !isTauri()}
          onClick={() => void openLink()}
          className="shrink-0 self-start sm:self-auto"
        >
          <Link2 className="w-3.5 h-3.5" />
          <span>Link playlists</span>
        </Button>
      </div>

      {error && (
        <div
          role="alert"
          className="p-3.5 rounded-xl border border-rose-500/40 bg-rose-950/25 text-xs text-rose-300 flex items-start gap-2.5"
        >
          <AlertTriangle className="w-4 h-4 text-rose-400 shrink-0 mt-0.5" />
          <span className="leading-relaxed">{error}</span>
        </div>
      )}

      {/* Linking Card */}
      {linking && (
        <Card elevated className="p-5 space-y-4 border-white/[0.1]">
          <div className="flex items-start justify-between gap-3">
            <div>
              <h2 className="text-sm font-semibold text-zinc-100">
                Choose existing editable playlists
              </h2>
              <p className="text-xs text-zinc-400 mt-0.5">
                The first merge requires review. Names, descriptions, artwork, and visibility stay local.
              </p>
            </div>
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            {([0, 1] as const).map((side) => {
              const isSpotify = side === 0;
              const filterVal = isSpotify ? spotifyFilter : youtubeFilter;
              const setFilterVal = isSpotify ? setSpotifyFilter : setYoutubeFilter;
              const list = filteredCatalog[side];

              return (
                <div key={side} className="p-3.5 rounded-xl bg-[#09090d] border border-white/[0.08] space-y-2.5">
                  <div className="flex items-center justify-between gap-2">
                    <div className="flex items-center gap-2 text-xs font-semibold text-zinc-200">
                      <ProviderLogo provider={isSpotify ? "spotify" : "ytmusic"} size="sm" />
                      <span>{isSpotify ? "Spotify" : "YouTube Music"}</span>
                    </div>
                    <span className="text-[10px] font-mono text-zinc-500 tabular-nums">
                      {list.length} available
                    </span>
                  </div>

                  <SearchInput
                    value={filterVal}
                    onChange={(e) => setFilterVal(e.target.value)}
                    onClear={() => setFilterVal("")}
                    placeholder={`Filter ${isSpotify ? "Spotify" : "YouTube Music"} playlists...`}
                  />

                  <label className="block space-y-1 text-xs">
                    <span className="sr-only">{isSpotify ? "Spotify" : "YouTube Music"}</span>
                    <select
                      aria-label={isSpotify ? "Spotify playlist" : "YouTube Music playlist"}
                      className="block w-full bg-[#121217] border border-white/[0.1] rounded-lg px-3 py-2 text-xs text-zinc-200 focus:outline-none focus:border-emerald-500/60"
                      value={isSpotify ? spotifyId : youtubeId}
                      onChange={(e) =>
                        isSpotify ? setSpotifyId(e.target.value) : setYoutubeId(e.target.value)
                      }
                      disabled={busy}
                    >
                      <option value="">Choose a playlist</option>
                      {list.map((p) => (
                        <option value={p.id} key={p.id}>
                          {p.title}
                        </option>
                      ))}
                    </select>
                  </label>
                </div>
              );
            })}
          </div>

          <div className="flex items-center gap-2 pt-1">
            <Button
              variant="primary"
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
        <Card className="p-4 border-dashed border-white/[0.1] text-xs text-zinc-400">
          Open the desktop app to connect and sync playlists.
        </Card>
      )}

      {/* Live Run Card */}
      {run && (
        <Card elevated className="p-4 space-y-3 border-emerald-500/25 bg-[#0b0e0d]" aria-live="polite">
          <div className="flex items-center justify-between gap-3 flex-wrap">
            <div className="flex items-center gap-2">
              <Badge variant={syncStateBadgeVariant(run.state, run.verified)}>
                {run.verified ? "Both playlists verified" : run.state.replaceAll("_", " ")}
              </Badge>
              {syncProgress?.track_title && (
                <span className="text-xs text-zinc-300 truncate max-w-md">
                  {syncProgress.side ? `[${syncProgress.side}] ` : ""}
                  {syncProgress.track_title}
                </span>
              )}
            </div>
            <p className="text-xs font-mono text-zinc-400 tabular-nums">
              {run.counts.added} added · {run.counts.removed} removed ·{" "}
              {run.counts.moved} moved · {run.counts.uncertain} uncertain
            </p>
          </div>

          {progressPct !== null && (
            <div className="space-y-1">
              <div className="flex justify-between text-[11px] font-mono text-zinc-400 tabular-nums">
                <span>Applying remote edits...</span>
                <span>{syncProgress!.processed} / {syncProgress!.total} ({progressPct}%)</span>
              </div>
              <div className="h-1.5 w-full rounded-full bg-white/[0.06] overflow-hidden">
                <div
                  className="h-full bg-emerald-500 transition-all duration-200"
                  style={{ width: `${progressPct}%` }}
                />
              </div>
            </div>
          )}

          {!terminal(run.state) && (
            <div className="flex gap-2 flex-wrap">
              <Button
                variant="secondary"
                size="sm"
                disabled={controlPending || run.state !== "running"}
                onClick={() => void requestControl("pause")}
              >
                <Pause className="w-3.5 h-3.5" />
                <span>Pause sync</span>
              </Button>
              <Button
                variant="secondary"
                size="sm"
                disabled={
                  controlPending || !["pausing", "paused"].includes(run.state)
                }
                onClick={() => void requestControl("resume")}
              >
                <Play className="w-3.5 h-3.5" />
                <span>Resume sync</span>
              </Button>
              <Button
                variant="danger"
                size="sm"
                disabled={
                  controlPending ||
                  ["starting", "cancelling"].includes(run.state)
                }
                onClick={() => void requestControl("cancel")}
              >
                <XSquare className="w-3.5 h-3.5" />
                <span>Cancel sync</span>
              </Button>
              {controlPending && (
                <span className="text-xs text-zinc-400 self-center">
                  Waiting for acknowledgment...
                </span>
              )}
            </div>
          )}
        </Card>
      )}

      {/* Linked Pairs List */}
      <div className="space-y-3">
        {pairs.map((pair) => {
          const isBusyState = ["running", "paused", "pausing", "cancelling", "checking"].includes(
            pair.state,
          );
          const badgeVariant = syncStateBadgeVariant(pair.state, pair.state === "synced");

          return (
            <Card key={pair.pair_id} className="p-4 space-y-3 bg-[#0b0b10] border-white/[0.08] hover:border-white/[0.14]">
              <div className="flex flex-col lg:flex-row lg:items-center justify-between gap-3">
                <div className="min-w-0 space-y-1">
                  <div className="flex items-center gap-2 flex-wrap">
                    <div className="flex items-center gap-2 text-sm font-semibold text-zinc-100">
                      <ProviderLogo provider="spotify" size="sm" />
                      <span>{pair.spotify.title} ↔ {pair.ytmusic.title}</span>
                      <ProviderLogo provider="ytmusic" size="sm" />
                    </div>
                    <Badge variant={badgeVariant}>
                      {pair.state.replaceAll("_", " ")}
                    </Badge>
                  </div>

                  <div className="flex items-center gap-2 text-xs text-zinc-500 font-mono">
                    <Clock className="w-3 h-3 shrink-0" />
                    <span>{pair.enabled ? formatDueCountdown(pair.next_due_at) : "Hourly schedule is off"}</span>
                  </div>
                </div>

                <div className="flex gap-2 flex-wrap shrink-0">
                  <Button
                    variant="primary"
                    size="sm"
                    disabled={busy || isBusyState}
                    onClick={() => void preview(pair.pair_id)}
                  >
                    <ArrowLeftRight className="w-3.5 h-3.5" />
                    <span>Sync now</span>
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
                    <Clock className="w-3.5 h-3.5" />
                    <span>{pair.enabled ? "Disable hourly sync" : "Enable hourly sync"}</span>
                  </Button>
                  {pair.last_run_id && (
                    <Button
                      size="sm"
                      variant="secondary"
                      disabled={busy}
                      onClick={() => void inspectRun(pair.last_run_id!)}
                    >
                      <Search className="w-3.5 h-3.5" />
                      <span>Inspect sync</span>
                    </Button>
                  )}
                  {pair.state === "authentication_required" && (
                    <Button
                      size="sm"
                      variant="amber"
                      disabled={busy}
                      onClick={() => void revalidate(pair.pair_id)}
                    >
                      <span>Revalidate account binding</span>
                    </Button>
                  )}
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={busy || isBusyState}
                    onClick={() => setUnlinking(pair)}
                  >
                    <Unlink className="w-3.5 h-3.5" />
                    <span>Unlink pair</span>
                  </Button>
                </div>
              </div>

              {pair.last_error && (
                <div className="p-2.5 rounded-lg bg-amber-950/25 border border-amber-500/25 text-xs text-amber-300 flex items-start gap-2">
                  <AlertTriangle className="w-3.5 h-3.5 text-amber-400 shrink-0 mt-0.5" />
                  <span>{pair.last_error}</span>
                </div>
              )}
            </Card>
          );
        })}
      </div>

      {/* Sync Run Inspection Card */}
      {inspection && (
        <Card elevated className="p-4 space-y-3 border-white/[0.1]">
          <div className="flex items-center justify-between gap-3 flex-wrap">
            <div className="flex items-center gap-2">
              <ShieldCheck className="w-4 h-4 text-emerald-400" />
              <h2 className="text-sm font-semibold text-zinc-100">Recorded sync inspection</h2>
            </div>
            <span className="text-xs font-mono text-zinc-400 tabular-nums">
              {inspection.summary.counts.failed} failed ·{" "}
              {inspection.summary.counts.uncertain} uncertain ·{" "}
              {inspection.summary.counts.unsent} unsent
            </span>
          </div>

          <p className="text-xs text-zinc-300">{inspection.next_action}</p>

          <ul className="space-y-2 max-h-60 overflow-y-auto pr-1 scrollbar-thin scrollbar-thumb-zinc-800">
            {inspection.operations.map((operation) => (
              <li
                key={operation.operation_id}
                className="p-2 rounded-lg bg-[#09090d] border border-white/[0.06] text-xs text-zinc-300 font-mono"
              >
                <div>
                  {operation.side} · {operation.kind} · {operation.status} ·{" "}
                  {operation.attempt_count} attempts
                </div>
                {operation.detail && (
                  <p className="mt-1 text-amber-300 font-sans">{operation.detail}</p>
                )}
              </li>
            ))}
          </ul>

          <div className="flex gap-2 flex-wrap pt-1">
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
              variant="primary"
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

      {/* Empty State */}
      {pairs.length === 0 && (
        <Card className="p-10 border-dashed border-white/[0.1] bg-[#09090c]/40 text-center space-y-2">
          <RefreshCw className="w-8 h-8 text-zinc-600 mx-auto" />
          <p className="text-xs text-zinc-400">
            Link two existing playlists to review their initial merge.
          </p>
        </Card>
      )}

      {/* Merge Review */}
      {plan && (
        <SyncReview
          key={plan.plan_id}
          plan={plan}
          busy={busy}
          onResolve={(resolution) => void preview(plan.pair_id, resolution)}
          onApply={() => void apply()}
        />
      )}

      {/* Unlink Modal */}
      <Modal
        isOpen={unlinking !== null}
        onClose={() => {
          if (!busy) setUnlinking(null);
        }}
        disableDismiss={busy}
        title="Unlink playlist pair"
        description="Both playlists and recorded run history are preserved."
        accent="danger"
        footer={
          <>
            <Button variant="secondary" disabled={busy} onClick={() => setUnlinking(null)}>
              Keep linked
            </Button>
            <Button variant="danger" disabled={busy} onClick={() => void unlink()}>
              Confirm unlink
            </Button>
          </>
        }
      >
        <p className="text-xs text-zinc-400 leading-relaxed">
          Hourly checks stop for this pair. Unlinking does not resolve uncertain recorded writes.
        </p>
      </Modal>
    </section>
  );
}
