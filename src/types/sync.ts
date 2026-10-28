import { SourceTrack } from "./provider";
export type SyncSide = "spotify" | "ytmusic";
export interface LinkedOccurrence {
  logical_id: string;
  spotify: SourceTrack | null;
  ytmusic: SourceTrack | null;
  spotify_remote_id: string | null;
  ytmusic_remote_id: string | null;
}
export interface PlaylistRead {
  complete: boolean;
  revision: string | null;
  occurrences: {
    index: number;
    track: SourceTrack;
    remote_id: string | null;
  }[];
}
export interface SyncPair {
  pair_id: string;
  spotify: { playlist_id: string; title: string; generation: number };
  ytmusic: { playlist_id: string; title: string; generation: number };
  baseline: {
    version: number;
    occurrences: LinkedOccurrence[];
    spotify: PlaylistRead;
    ytmusic: PlaylistRead;
  } | null;
  enabled: boolean;
  state: string;
  next_due_at: number | null;
  last_run_id: string | null;
  last_error: string | null;
}
export interface SyncConflict {
  code: string;
  logical_id: string | null;
  detail: string;
  candidates: SourceTrack[];
}
export interface SyncPlan {
  plan_id: string;
  pair_id: string;
  baseline_version: number;
  pair_stamp: string;
  spotify_before: PlaylistRead;
  ytmusic_before: PlaylistRead;
  desired: LinkedOccurrence[];
  conflicts: SyncConflict[];
  initial: boolean;
}
export interface SyncResolution {
  plan_id: string;
  order: string[];
  choices: { logical_id: string; side: SyncSide; track: SourceTrack }[];
  strategy: SyncSide | null;
}
export interface SyncCounts {
  added: number;
  removed: number;
  moved: number;
  failed: number;
  uncertain: number;
  unsent: number;
}
export interface SyncUpdate {
  pair_id: string;
  job_id: string;
  state: string;
  event_seq: number;
}
export interface SyncRunSummary extends SyncUpdate {
  counts: SyncCounts;
  verified: boolean;
  error: string | null;
}
export interface SyncInspection {
  summary: SyncRunSummary;
  operations: {
    operation_id: string;
    side: SyncSide;
    kind: string;
    status: string;
    attempt_count: number;
    detail: string | null;
  }[];
  can_resume: boolean;
  next_action: string;
}
