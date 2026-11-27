import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowUp, ArrowDown, AlertTriangle, CheckCircle2, Music, Trash2 } from "lucide-react";
import { Badge, Button, Card, SearchInput } from "../ui";
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
  const [filterQuery, setFilterQuery] = useState("");
  const [jumpTarget, setJumpTarget] = useState<Record<string, string>>({});
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

  const moveToPosition = (fromIndex: number, logicalId: string) => {
    const raw = Number.parseInt(jumpTarget[logicalId] || "", 10);
    if (!Number.isFinite(raw)) return;
    const toIndex = Math.min(order.length - 1, Math.max(0, raw - 1));
    if (toIndex === fromIndex) return;
    const next = [...order];
    const [moved] = next.splice(fromIndex, 1);
    next.splice(toIndex, 0, moved);
    setOrder(next);
    setJumpTarget((prev) => ({ ...prev, [logicalId]: "" }));
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
      plan[side === "spotify" ? "spotify_before" : "ytmusic_before"].occurrences;
    const remaining = new Map<string, number>();
    for (const id of order) {
      const o = plan.desired.find((item) => item.logical_id === id)!;
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

  const indexedRows = useMemo(() => {
    const q = filterQuery.trim().toLowerCase();
    const mapped = order.map((id, index) => ({
      id,
      index,
      occurrence: plan.desired.find((o) => o.logical_id === id)!,
    }));
    if (!q) return mapped.slice(0, visible);
    return mapped.filter(({ occurrence }) => {
      const spTitle = occurrence.spotify?.title?.toLowerCase() ?? "";
      const ytTitle = occurrence.ytmusic?.title?.toLowerCase() ?? "";
      const artists = [
        ...(occurrence.spotify?.artists ?? []),
        ...(occurrence.ytmusic?.artists ?? []),
      ]
        .join(" ")
        .toLowerCase();
      return spTitle.includes(q) || ytTitle.includes(q) || artists.includes(q);
    });
  }, [order, plan.desired, filterQuery, visible]);

  return (
    <Card elevated className="p-5 space-y-4 border-white/[0.1]">
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 pb-3 border-b border-white/[0.08]">
        <div>
          <h2 className="text-sm font-semibold text-zinc-100">
            {plan.initial ? "Review initial merge" : "Review playlist changes"}
          </h2>
          <p className="text-xs text-zinc-400 mt-0.5">
            The resulting song order applies to both playlists. Repeated songs remain separate entries.
          </p>
        </div>

        <SearchInput
          value={filterQuery}
          onChange={(e) => setFilterQuery(e.target.value)}
          onClear={() => setFilterQuery("")}
          placeholder={`Filter ${order.length} tracks...`}
          className="w-full sm:w-60 shrink-0"
        />
      </div>

      {/* Diff Summary Grid */}
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
        {edits.map((edit) => (
          <div
            key={edit.side}
            className="rounded-xl border border-white/[0.08] bg-[#08080c] p-3 text-xs text-zinc-400"
          >
            <div className="flex items-center justify-between">
              <span className="font-semibold text-zinc-200">
                {edit.side === "spotify" ? "Spotify" : "YouTube Music"}
              </span>
              <span className="font-mono tabular-nums">
                <strong className="text-emerald-400">{edit.added}</strong> additions ·{" "}
                <strong className={edit.removed.length > 0 ? "text-amber-300" : "text-zinc-300"}>
                  {edit.removed.length}
                </strong>{" "}
                removals
              </span>
            </div>
            {edit.removed.length > 0 && (
              <details className="mt-2 text-amber-300">
                <summary className="cursor-pointer font-medium">Review removals</summary>
                <ul className="max-h-40 overflow-y-auto mt-2 space-y-1 text-zinc-300">
                  {edit.removed.map((t, i) => (
                    <li key={i} className="truncate">
                      {t.title} - {t.artists.join(", ")}
                    </li>
                  ))}
                </ul>
              </details>
            )}
          </div>
        ))}
      </div>

      {order.length === 0 && (
        <p className="text-xs text-amber-300">
          Applying this review will empty both playlists.
        </p>
      )}

      {plan.conflicts.map((conflict, index) => (
        <div
          key={index}
          className="p-2.5 rounded-lg bg-amber-950/25 border border-amber-500/25 text-xs text-amber-300 flex items-start gap-2"
        >
          <AlertTriangle className="w-3.5 h-3.5 text-amber-400 shrink-0 mt-0.5" />
          <span>{conflict.detail}</span>
        </div>
      ))}

      {plan.conflicts.some((c) => c.code !== "match_required") && (
        <div className="flex gap-2 flex-wrap">
          <Button variant="secondary" size="sm" disabled={busy} onClick={() => resolve("spotify")}>
            Keep Spotify edits
          </Button>
          <Button variant="secondary" size="sm" disabled={busy} onClick={() => resolve("ytmusic")}>
            Keep YouTube Music edits
          </Button>
        </div>
      )}

      {error && (
        <p role="alert" className="text-xs text-rose-300 bg-rose-950/25 border border-rose-500/30 rounded-lg p-2.5">
          {error}
        </p>
      )}

      {/* Merged Song Order */}
      <ol
        className="divide-y divide-white/[0.06] max-h-[480px] overflow-y-auto pr-1 scrollbar-thin scrollbar-thumb-zinc-800 scrollbar-track-transparent rounded-xl border border-white/[0.08] bg-[#08080c]"
        aria-label="Merged song order"
      >
        {indexedRows.map(({ id, index, occurrence }) => {
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
          const primary = occurrence.spotify ?? occurrence.ytmusic;
          const thumb = primary?.thumbnail_url;

          return (
            <li
              key={id}
              className="p-3.5 space-y-2.5 hover:bg-white/[0.02] transition-colors"
              style={{ contentVisibility: "auto", containIntrinsicSize: "0 76px" }}
            >
              <div className="flex items-start justify-between gap-4">
                <div className="flex items-start gap-3 min-w-0">
                  {thumb ? (
                    <img
                      src={thumb}
                      alt=""
                      className="w-9 h-9 rounded-md object-cover shrink-0 bg-zinc-900 border border-white/[0.08]"
                      loading="lazy"
                    />
                  ) : (
                    <div className="w-9 h-9 rounded-md bg-zinc-900 border border-white/[0.08] shrink-0 flex items-center justify-center text-zinc-500">
                      <Music className="w-4 h-4" />
                    </div>
                  )}

                  <div className="min-w-0">
                    <div className="flex items-center gap-2 flex-wrap">
                      <p className="text-xs font-semibold text-zinc-100">
                        <span className="font-mono text-zinc-500 mr-1.5 tabular-nums">{index + 1}.</span>
                        {primary?.title}
                      </p>
                      {missing && !chosen ? (
                        <Badge variant="amber">Needs {missing === "spotify" ? "Spotify" : "YT Music"}</Badge>
                      ) : (
                        <Badge variant="exact" icon={<CheckCircle2 className="w-3 h-3 text-emerald-400" />}>
                          Aligned
                        </Badge>
                      )}
                    </div>
                    <p className="text-[11px] text-zinc-400 mt-1">
                      {occurrence.spotify?.title ??
                        (missing === "spotify" ? chosen?.title : undefined) ??
                        "Spotify match needed"}{" "}
                      ↔{" "}
                      {occurrence.ytmusic?.title ??
                        (missing === "ytmusic" ? chosen?.title : undefined) ??
                        "YouTube Music match needed"}
                    </p>
                  </div>
                </div>

                <div className="flex items-center gap-1 shrink-0">
                  {order.length > 3 && (
                    <form
                      onSubmit={(e) => {
                        e.preventDefault();
                        moveToPosition(index, id);
                      }}
                      className="hidden sm:flex items-center gap-1 mr-1"
                    >
                      <input
                        type="number"
                        min={1}
                        max={order.length}
                        value={jumpTarget[id] ?? ""}
                        onChange={(e) =>
                          setJumpTarget((prev) => ({ ...prev, [id]: e.target.value }))
                        }
                        placeholder="#"
                        aria-label={`Move song ${index + 1} to position`}
                        className="w-12 rounded-md border border-white/[0.08] bg-[#121217] px-1.5 py-1 text-[11px] font-mono text-zinc-200 text-center focus:outline-none focus:border-emerald-500/60"
                      />
                    </form>
                  )}
                  <button
                    type="button"
                    aria-label={`Move song ${index + 1} up`}
                    disabled={busy || index === 0}
                    onClick={() => move(index, -1)}
                    className="p-1.5 rounded-lg border border-white/[0.08] bg-[#121217] text-zinc-300 hover:bg-[#1a1a22] transition-colors cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed"
                  >
                    <ArrowUp size={14} />
                  </button>
                  <button
                    type="button"
                    aria-label={`Move song ${index + 1} down`}
                    disabled={busy || index === order.length - 1}
                    onClick={() => move(index, 1)}
                    className="p-1.5 rounded-lg border border-white/[0.08] bg-[#121217] text-zinc-300 hover:bg-[#1a1a22] transition-colors cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed"
                  >
                    <ArrowDown size={14} />
                  </button>
                </div>
              </div>

              {missing && (
                <div className="space-y-2 pl-12">
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
                      className="min-w-0 flex-1 rounded-lg border border-white/[0.08] bg-[#121217] px-2.5 py-1.5 text-xs text-zinc-200 placeholder:text-zinc-600 focus:outline-none focus:border-emerald-500/60"
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
                  {candidates.length > 0 && (
                    <div className="flex gap-1.5 flex-wrap">
                      {candidates.map((candidate) => (
                        <Button
                          key={candidate.id}
                          size="xs"
                          variant="secondary"
                          disabled={busy}
                          onClick={() => choose(id, missing, candidate)}
                        >
                          Use {candidate.title}
                        </Button>
                      ))}
                    </div>
                  )}
                </div>
              )}

              <div className="pl-12">
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => {
                    setOrder((current) =>
                      current.filter((logical) => logical !== id),
                    );
                    setDirty(true);
                  }}
                  className="inline-flex items-center gap-1 text-[11px] text-zinc-500 hover:text-amber-300 transition-colors cursor-pointer"
                >
                  <Trash2 className="w-3 h-3" />
                  <span>Remove this occurrence from both playlists</span>
                </button>
              </div>
            </li>
          );
        })}
      </ol>

      {!filterQuery.trim() && visible < order.length && (
        <Button
          size="sm"
          variant="secondary"
          onClick={() => setVisible((n) => n + 100)}
        >
          Show next 100 songs ({order.length - visible} remaining)
        </Button>
      )}

      <div className="flex items-center gap-2 flex-wrap pt-1">
        <Button variant="secondary" disabled={busy} onClick={() => resolve()}>
          Validate review
        </Button>
        <Button
          variant="primary"
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
