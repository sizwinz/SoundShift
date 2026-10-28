import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowUp, ArrowDown } from "lucide-react";
import { Button, Card } from "../ui";
import { SourceTrack } from "../../types/provider";
import { SyncPlan, SyncResolution, SyncSide } from "../../types/sync";

interface Props {
  plan: SyncPlan;
  busy: boolean;
  onResolve: (resolution: SyncResolution) => void;
  onApply: () => void;
}
export function SyncReview({ plan, busy, onResolve, onApply }: Props) {
  const [order, setOrder] = useState(plan.desired.map((o) => o.logical_id));
  const [choices, setChoices] = useState<SyncResolution["choices"]>([]);
  const [dirty, setDirty] = useState(false);
  const [search, setSearch] = useState<Record<string, string>>({});
  const [results, setResults] = useState<Record<string, SourceTrack[]>>({});
  const [searching, setSearching] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [visible, setVisible] = useState(100);
  const choose = (logical_id: string, side: SyncSide, track: SourceTrack) => {
    setChoices((current) => [
      ...current.filter((c) => c.logical_id !== logical_id || c.side !== side),
      { logical_id, side, track },
    ]);
    setDirty(true);
  };
  const move = (index: number, delta: number) => {
    const next = [...order];
    [next[index], next[index + delta]] = [next[index + delta], next[index]];
    setOrder(next);
    setDirty(true);
  };
  const resolve = (strategy: SyncSide | null = null) =>
    onResolve({ plan_id: plan.plan_id, order, choices, strategy });
  const find = async (logical: string, side: SyncSide) => {
    const query = search[logical]?.trim();
    if (!query) return;
    setSearching(logical);
    setError(null);
    try {
      const tracks = /^(https?:|spotify:track:)/.test(query)
        ? [
            await invoke<SourceTrack>("resolve_track_by_url", {
              service: side,
              url: query,
            }),
          ]
        : await invoke<SourceTrack[]>("search_provider_tracks", {
            service: side,
            query,
          });
      setResults((current) => ({ ...current, [logical]: tracks }));
    } catch (e) {
      setError(String(e));
    } finally {
      setSearching(null);
    }
  };
  const edits = (["spotify", "ytmusic"] as const).map((side) => {
    const before =
      plan[side === "spotify" ? "spotify_before" : "ytmusic_before"]
        .occurrences;
    const remaining = new Map<string, number>();
    for (const id of order) {
      const o = plan.desired.find((o) => o.logical_id === id)!;
      const chosen =
        choices.find((c) => c.logical_id === id && c.side === side)?.track ??
        o[side];
      if (chosen) remaining.set(chosen.id, (remaining.get(chosen.id) ?? 0) + 1);
    }
    const removed: SourceTrack[] = [];
    for (const o of before) {
      const count = remaining.get(o.track.id) ?? 0;
      if (count > 0) remaining.set(o.track.id, count - 1);
      else removed.push(o.track);
    }
    return {
      side,
      removed,
      added: [...remaining.values()].reduce((sum, n) => sum + n, 0),
    };
  });
  return (
    <Card className="p-4 space-y-4">
      <h2 className="text-base font-medium">
        {plan.initial ? "Review initial merge" : "Review playlist changes"}
      </h2>
      <p className="text-sm text-zinc-400">
        The resulting song order applies to both playlists. Repeated songs
        remain separate entries.
      </p>
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
        {edits.map((edit) => (
          <div
            key={edit.side}
            className="rounded-md border border-white/10 p-3 text-xs text-zinc-400"
          >
            <span className="text-zinc-200">
              {edit.side === "spotify" ? "Spotify" : "YouTube Music"}
            </span>
            : {edit.added} additions · {edit.removed.length} removals
            {edit.removed.length > 0 && (
              <details className="mt-2 text-amber-300">
                <summary className="cursor-pointer">Review removals</summary>
                <ul className="max-h-40 overflow-y-auto mt-2 space-y-1">
                  {edit.removed.map((t, i) => (
                    <li key={i}>
                      {t.title} — {t.artists.join(", ")}
                    </li>
                  ))}
                </ul>
              </details>
            )}
          </div>
        ))}
      </div>
      {order.length === 0 && (
        <p className="text-sm text-amber-300">
          Applying this review will empty both playlists.
        </p>
      )}
      {plan.conflicts.map((conflict, index) => (
        <p key={index} className="text-sm text-amber-300">
          {conflict.detail}
        </p>
      ))}
      {plan.conflicts.some((c) => c.code !== "match_required") && (
        <div className="flex gap-2 flex-wrap">
          <Button size="sm" disabled={busy} onClick={() => resolve("spotify")}>
            Keep Spotify edits
          </Button>
          <Button size="sm" disabled={busy} onClick={() => resolve("ytmusic")}>
            Keep YouTube Music edits
          </Button>
        </div>
      )}
      {error && (
        <p role="alert" className="text-sm text-red-300">
          {error}
        </p>
      )}
      <ol
        className="divide-y divide-white/10 max-h-[460px] overflow-y-auto pr-2"
        aria-label="Merged song order"
      >
        {order.slice(0, visible).map((id, index) => {
          const occurrence = plan.desired.find((o) => o.logical_id === id)!;
          const missing: SyncSide | null = !occurrence.spotify
            ? "spotify"
            : !occurrence.ytmusic
              ? "ytmusic"
              : null;
          const chosen = choices.find(
            (c) => c.logical_id === id && c.side === missing,
          )?.track;
          const candidates =
            results[id] ??
            plan.conflicts.find((c) => c.logical_id === id)?.candidates ??
            [];
          return (
            <li key={id} className="py-3 space-y-2">
              <div className="flex items-start justify-between gap-4">
                <div className="min-w-0">
                  <p className="text-sm">
                    {index + 1}.{" "}
                    {occurrence.spotify?.title ?? occurrence.ytmusic?.title}
                  </p>
                  <p className="text-xs text-zinc-500 mt-1">
                    {occurrence.spotify?.title ??
                      (missing === "spotify" ? chosen?.title : undefined) ??
                      "Spotify match needed"}{" "}
                    ↔{" "}
                    {occurrence.ytmusic?.title ??
                      (missing === "ytmusic" ? chosen?.title : undefined) ??
                      "YouTube Music match needed"}
                  </p>
                </div>
                <div className="flex gap-1 shrink-0">
                  <button
                    type="button"
                    aria-label={`Move song ${index + 1} up`}
                    disabled={busy || index === 0}
                    onClick={() => move(index, -1)}
                    className="p-1.5 rounded border border-zinc-700 disabled:opacity-30"
                  >
                    <ArrowUp size={14} />
                  </button>
                  <button
                    type="button"
                    aria-label={`Move song ${index + 1} down`}
                    disabled={busy || index === order.length - 1}
                    onClick={() => move(index, 1)}
                    className="p-1.5 rounded border border-zinc-700 disabled:opacity-30"
                  >
                    <ArrowDown size={14} />
                  </button>
                </div>
              </div>
              {missing && (
                <div className="space-y-2">
                  <div className="flex gap-2">
                    <input
                      aria-label={`Find a ${missing} match for song ${index + 1}`}
                      placeholder="Search title and artist, or paste a track URL"
                      value={search[id] ?? ""}
                      onChange={(e) =>
                        setSearch((current) => ({
                          ...current,
                          [id]: e.target.value,
                        }))
                      }
                      className="min-w-0 flex-1 rounded-md border border-zinc-700 bg-zinc-950 p-2 text-xs"
                    />
                    <Button
                      size="sm"
                      disabled={
                        busy || searching !== null || !search[id]?.trim()
                      }
                      onClick={() => void find(id, missing)}
                    >
                      Search
                    </Button>
                  </div>
                  <div className="flex gap-2 flex-wrap">
                    {candidates.map((candidate) => (
                      <Button
                        key={candidate.id}
                        size="sm"
                        variant="secondary"
                        disabled={busy}
                        onClick={() => choose(id, missing, candidate)}
                      >
                        Use {candidate.title}
                      </Button>
                    ))}
                  </div>
                </div>
              )}
              <button
                type="button"
                disabled={busy}
                onClick={() => {
                  setOrder((current) =>
                    current.filter((logical) => logical !== id),
                  );
                  setDirty(true);
                }}
                className="text-xs text-zinc-500 hover:text-amber-300"
              >
                Remove this occurrence from both playlists
              </button>
            </li>
          );
        })}
      </ol>
      {visible < order.length && (
        <Button
          size="sm"
          variant="secondary"
          onClick={() => setVisible((n) => n + 100)}
        >
          Show next 100 songs ({order.length - visible} remaining)
        </Button>
      )}
      <div className="flex gap-2 flex-wrap">
        <Button variant="secondary" disabled={busy} onClick={() => resolve()}>
          Validate review
        </Button>
        <Button
          onClick={onApply}
          disabled={
            busy ||
            dirty ||
            plan.conflicts.length > 0 ||
            plan.desired.some((o) => !o.spotify || !o.ytmusic)
          }
        >
          Apply reviewed sync
        </Button>
      </div>
      {dirty && (
        <p className="text-xs text-amber-300">
          Validate your changes against fresh playlist reads before applying.
        </p>
      )}
    </Card>
  );
}
