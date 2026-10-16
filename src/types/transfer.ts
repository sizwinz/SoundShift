import { SourceTrack } from "./provider";

export type TransferStage =
  | "idle"
  | "starting"
  | "transferring"
  | "pausing"
  | "paused"
  | "cancelling"
  | "recovery_required"
  | "completed"
  | "cancelled"
  | "failed";

export interface LifecycleUpdate {
  job_id: string;
  state: "starting" | "running" | "pausing" | "paused" | "cancelling" | "cancelled" | "completed_verified" | "recovery_required";
  event_seq: number;
}

export interface TransferProgressPayload {
  job_id: string;
  stage: string;
  processed: number;
  total: number;
  successful: number;
  failed: number;
  current_track: string | null;
  worker_id: number;
  latency_ms: number;
  http_status: number;
  is_retry: boolean;
}

export interface TransferLogPayload {
  job_id: string;
  timestamp: number;
  worker_id: number;
  message: string;
  level: "info" | "warn" | "error";
}

export interface AuditResult {
  is_verified: boolean;
  total_expected: number;
  total_found: number;
  missing_ids: string[];
}

export interface BatchTransferConfig {
  job_id: string;
  source_service: string;
  target_service: string;
  source_playlist_name: string;
  target_playlist_id?: string | null;
  target_playlist_name?: string | null;
  is_new_playlist: boolean;
  tracks: SourceTrack[];
  concurrency: number;
  skip_duplicates?: boolean;
}

export interface BatchTransferSummary {
  job_id: string;
  snapshot_id: string;
  target_playlist_id: string;
  total_tracks: number;
  successful_tracks: number;
  failed_tracks: number;
  skipped_duplicates?: number;
  is_cancelled: boolean;
  audit: AuditResult | null;
  state: LifecycleUpdate["state"];
  event_seq: number;
}

export interface TransferHistoryEntry {
  job_id: string;
  snapshot_id: string;
  source_service: string;
  target_service: string;
  source_playlist_name: string;
  target_playlist_id: string;
  is_new_playlist: boolean;
  total_tracks: number;
  matched_tracks: number;
  added_tracks_count: number;
  pre_existing_count: number;
  is_rolled_back: boolean;
  created_at: number;
  audit_status?: AuditResult | null;
  job_state: string;
  recovery_required: boolean;
  last_error: string | null;
}
