import { SourceTrack } from "./provider";

export type MatchClassification = "Exact" | "Ambiguous" | "NotFound";

export interface MatchCandidate {
  track: SourceTrack;
  similarity: number;
  duration_delta_ms: number;
}

export interface MatchResult {
  source_track: SourceTrack;
  status: MatchClassification;
  matched_track?: SourceTrack | null;
  candidates: MatchCandidate[];
  confidence: number;
  match_method?: string | null;
}

export interface MatchingProgressPayload {
  playlist_id: string;
  processed: number;
  total: number;
  current_result: MatchResult;
}
