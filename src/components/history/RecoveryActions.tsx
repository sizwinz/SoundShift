import {useRef,useState} from "react";
import {invoke} from "@tauri-apps/api/core";
import {JobInspection} from "../../types/transfer";
import {Button} from "../ui";

export function RecoveryActions({jobId,onChanged,onRollback}:{jobId:string;onChanged:(value:JobInspection)=>void;onRollback:()=>void}) {
  const [inspection,setInspection]=useState<JobInspection|null>(null);
  const [busy,setBusy]=useState(false);
  const [error,setError]=useState<string|null>(null);
  const pending=useRef(false);
  async function request(command:string) {
    if(pending.current)return;
    pending.current=true;setBusy(true);setError(null);
    try {
      const value=await invoke<JobInspection>(command,{jobId});
      if(value.job_id!==jobId)throw new Error("Recovery response does not match the selected job");
      setInspection(previous=>previous && previous.event_seq>value.event_seq ? previous : value);
      onChanged(value);
    } catch(reason) {
      setError(String(reason));
      try {const value=await invoke<JobInspection>("inspect_transfer_job",{jobId});if(value.job_id===jobId){setInspection(value);onChanged(value);}} catch { /* Original actionable error stays visible. */ }
    } finally {pending.current=false;setBusy(false);}
  }
  return <div className="mt-3 border-t border-white/10 pt-3 text-xs text-zinc-300" aria-busy={busy}>
    <Button variant="secondary" size="xs" disabled={busy} onClick={()=>request("inspect_transfer_job")}>Inspect recovery</Button>
    {inspection && <div className="mt-2 space-y-2" aria-live="polite">
      <p>{inspection.counts.selected} selected · {inspection.counts.confirmed} confirmed · {inspection.counts.failed} failed · {inspection.counts.skipped} skipped · {inspection.counts.uncertain} uncertain · {inspection.counts.unsent} unsent</p>
      <p>State: {inspection.state.replaceAll("_"," ")} · Verification: {inspection.verification}</p>
      <p>{inspection.next_action}</p>
      <div className="flex gap-2">
        <Button variant="secondary" size="xs" disabled={busy || ["completed_verified","rolled_back"].includes(inspection.state)} onClick={()=>request("reconcile_transfer_job")}>Reconcile uncertain</Button>
        <Button variant="secondary" size="xs" disabled={busy || !inspection.can_resume} onClick={()=>request("resume_recorded_transfer")}>Resume recorded operations</Button>
        <Button variant="danger" size="xs" disabled={busy || inspection.counts.confirmed===0 || inspection.counts.uncertain>0 || inspection.state==="rolled_back"} onClick={onRollback}>Rollback recorded additions</Button>
      </div>
      <ul>{inspection.operations.slice(0,100).map(operation=><li key={operation.operation_id}>{operation.operation_id}: {operation.status}, {operation.attempt_count} attempt(s)</li>)}</ul>
      {inspection.operations.length>100 && <p>{inspection.operations.length-100} additional operations recorded.</p>}
    </div>}
    {busy && <p role="status">Waiting for backend recovery acknowledgment…</p>}
    {error && <p role="alert" className="mt-2 text-rose-300">{error}</p>}
  </div>;
}
