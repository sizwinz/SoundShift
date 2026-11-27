import { useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { JobInspection } from "../../types/transfer";
import { Badge, BadgeVariant, Button } from "../ui";

function inspectionStateVariant(state: string): BadgeVariant {
  if (state === "completed_verified") return "exact";
  if (state === "rolled_back") return "duplicate";
  if (["recovery_required", "manual_review"].includes(state)) return "unmatched";
  return "amber";
}

export function RecoveryActions({
  jobId,
  onChanged,
  onRollback,
}: {
  jobId: string;
  onChanged: (value: JobInspection) => void;
  onRollback: () => void;
}) {
  const [inspection, setInspection] = useState<JobInspection | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const pending = useRef(false);

  async function request(command: string) {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    setError(null);
    try {
      const value = await invoke<JobInspection>(command, { jobId });
      if (value.job_id !== jobId) {
        throw new Error("Recovery response does not match the selected job");
      }
      setInspection((previous) =>
        previous && previous.event_seq > value.event_seq ? previous : value
      );
      onChanged(value);
    } catch (reason) {
      setError(String(reason));
      try {
        const value = await invoke<JobInspection>("inspect_transfer_job", { jobId });
        if (value.job_id === jobId) {
          setInspection(value);
          onChanged(value);
        }
      } catch {
        // Original actionable error stays visible.
      }
    } finally {
      pending.current = false;
      setBusy(false);
    }
  }

  return (
    <div className="mt-3 border-t border-white/[0.08] pt-3 text-xs text-zinc-300" aria-busy={busy}>
      <div className="flex items-center justify-between gap-2 flex-wrap">
        <Button
          variant="secondary"
          size="xs"
          disabled={busy}
          onClick={() => request("inspect_transfer_job")}
        >
          Inspect recovery
        </Button>

        {inspection && (
          <Badge variant={inspectionStateVariant(inspection.state)}>
            {inspection.state.replaceAll("_", " ")} · Verification: {inspection.verification}
          </Badge>
        )}
      </div>

      {inspection && (
        <div
          className="mt-3 rounded-xl border border-white/[0.08] bg-[#08080c] p-3.5 space-y-3"
          aria-live="polite"
        >
          <div className="flex flex-wrap gap-2 text-[11px] font-mono tabular-nums">
            <span className="px-2 py-0.5 rounded bg-white/[0.04] text-zinc-300 border border-white/[0.06]">
              {inspection.counts.selected} selected
            </span>
            <span className="px-2 py-0.5 rounded bg-emerald-950/40 text-emerald-300 border border-emerald-500/20">
              {inspection.counts.confirmed} confirmed
            </span>
            <span className="px-2 py-0.5 rounded bg-rose-950/40 text-rose-300 border border-rose-500/20">
              {inspection.counts.failed} failed
            </span>
            <span className="px-2 py-0.5 rounded bg-white/[0.04] text-zinc-400 border border-white/[0.06]">
              {inspection.counts.skipped} skipped
            </span>
            <span
              className={`px-2 py-0.5 rounded border ${
                inspection.counts.uncertain > 0
                  ? "bg-amber-950/50 text-amber-300 border-amber-500/30 font-semibold"
                  : "bg-white/[0.04] text-zinc-400 border-white/[0.06]"
              }`}
            >
              {inspection.counts.uncertain} uncertain
            </span>
            <span className="px-2 py-0.5 rounded bg-white/[0.04] text-zinc-400 border border-white/[0.06]">
              {inspection.counts.unsent} unsent
            </span>
          </div>

          <p className="text-xs text-zinc-300 leading-relaxed">{inspection.next_action}</p>

          <div className="flex flex-wrap gap-2">
            <Button
              variant="secondary"
              size="xs"
              disabled={busy || ["completed_verified", "rolled_back"].includes(inspection.state)}
              onClick={() => request("reconcile_transfer_job")}
            >
              Reconcile uncertain
            </Button>
            <Button
              variant="primary"
              size="xs"
              disabled={busy || !inspection.can_resume}
              onClick={() => request("resume_recorded_transfer")}
            >
              Resume recorded operations
            </Button>
            <Button
              variant="danger"
              size="xs"
              disabled={
                busy ||
                inspection.counts.confirmed === 0 ||
                inspection.counts.uncertain > 0 ||
                inspection.state === "rolled_back"
              }
              onClick={onRollback}
            >
              Rollback recorded additions
            </Button>
          </div>

          {inspection.operations.length > 0 && (
            <ul className="max-h-44 overflow-y-auto space-y-1 font-mono text-[11px] text-zinc-400 pr-1 scrollbar-thin scrollbar-thumb-zinc-800">
              {inspection.operations.slice(0, 100).map((operation) => (
                <li
                  key={operation.operation_id}
                  className="px-2 py-1 rounded bg-[#0e0e13] border border-white/[0.04]"
                >
                  {operation.operation_id}: {operation.status}, {operation.attempt_count} attempt(s)
                </li>
              ))}
            </ul>
          )}

          {inspection.operations.length > 100 && (
            <p className="text-[11px] text-zinc-500">
              {inspection.operations.length - 100} additional operations recorded.
            </p>
          )}
        </div>
      )}

      {busy && (
        <p role="status" className="mt-2 text-xs text-zinc-400">
          Waiting for backend recovery acknowledgment...
        </p>
      )}
      {error && (
        <p role="alert" className="mt-2 text-rose-300 text-xs">
          {error}
        </p>
      )}
    </div>
  );
}
