export interface SourceTrack {
  id: string;
  title: string;
  artists: string[];
  album?: string | null;
  duration_ms: number;
  isrc?: string | null;
  is_explicit: boolean;
  is_playable: boolean;
  preview_url?: string | null;
  thumbnail_url?: string | null;
}

export interface Playlist {
  id: string;
  service: string;
  title: string;
  description?: string | null;
  track_count: number;
  is_public: boolean;
  cover_url?: string | null;
}

export interface IngestProgressPayload {
  playlist_id: string;
  loaded: number;
  total: number;
  tracks: SourceTrack[];
}
