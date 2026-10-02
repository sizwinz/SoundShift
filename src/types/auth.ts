export type ServiceId = 'ytmusic' | 'spotify';

export type AuthStatus = 'connected' | 'expired' | 'disconnected';

export interface AuthState {
  service: ServiceId;
  status: AuthStatus;
}

export interface ServiceAccount {
  id: ServiceId;
  name: string;
  description: string;
  status: AuthStatus;
  lastChecked?: number;
}
