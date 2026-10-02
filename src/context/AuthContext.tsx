import { createContext, useContext, useState, useEffect, useCallback, ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ServiceId, AuthStatus, ServiceAccount } from '../types/auth';

const INITIAL_ACCOUNTS: ServiceAccount[] = [
  {
    id: 'ytmusic',
    name: 'YouTube Music',
    description: 'Transfer library and playlists with automated SAPISIDHASH auth',
    status: 'disconnected',
  },
  {
    id: 'spotify',
    name: 'Spotify',
    description: 'Export curated playlists and tracks with isolated sp_dc auth',
    status: 'disconnected',
  },
];

interface AuthContextType {
  accounts: ServiceAccount[];
  loading: Record<ServiceId, boolean>;
  connect: (service: ServiceId) => Promise<void>;
  disconnect: (service: ServiceId) => Promise<void>;
  checkStatus: (service?: ServiceId) => Promise<void>;
  isPausedForAuth: boolean;
  expiredService: ServiceId | null;
  ytStatus: AuthStatus;
  spotifyStatus: AuthStatus;
}

const AuthContext = createContext<AuthContextType | null>(null);

export function AuthProvider({ children }: { children: ReactNode }) {
  const [accounts, setAccounts] = useState<ServiceAccount[]>(INITIAL_ACCOUNTS);
  const [loading, setLoading] = useState<Record<ServiceId, boolean>>({
    ytmusic: false,
    spotify: false,
  });

  const checkStatus = useCallback(async (service?: ServiceId) => {
    const servicesToCheck: ServiceId[] = service ? [service] : ['ytmusic', 'spotify'];

    for (const s of servicesToCheck) {
      setLoading((prev) => ({ ...prev, [s]: true }));
      try {
        const result = await invoke<{ service: string; status: AuthStatus }>('check_auth', {
          service: s,
        });
        setAccounts((prev) =>
          prev.map((acc) =>
            acc.id === s
              ? { ...acc, status: result.status, lastChecked: Date.now() }
              : acc
          )
        );
      } catch (err) {
        console.error(`Failed to check auth for ${s}:`, err);
      } finally {
        setLoading((prev) => ({ ...prev, [s]: false }));
      }
    }
  }, []);

  const connect = useCallback(async (service: ServiceId) => {
    setLoading((prev) => ({ ...prev, [service]: true }));
    try {
      await invoke('open_auth_window', { service });
    } catch (err) {
      console.error(`Failed to open auth window for ${service}:`, err);
      setLoading((prev) => ({ ...prev, [service]: false }));
    }
  }, []);

  const disconnect = useCallback(async (service: ServiceId) => {
    setLoading((prev) => ({ ...prev, [service]: true }));
    try {
      await invoke('disconnect_account', { service });
      setAccounts((prev) =>
        prev.map((acc) =>
          acc.id === service
            ? { ...acc, status: 'disconnected', lastChecked: Date.now() }
            : acc
        )
      );
    } catch (err) {
      console.error(`Failed to disconnect ${service}:`, err);
    } finally {
      setLoading((prev) => ({ ...prev, [service]: false }));
    }
  }, []);

  useEffect(() => {
    // Startup background validation per D-07
    checkStatus();

    // Event listener for backend auth changes per D-02
    let unlisten: (() => void) | undefined;
    listen<{ service: ServiceId; status: AuthStatus }>('auth:status_changed', (event) => {
      const { service, status } = event.payload;
      setAccounts((prev) =>
        prev.map((acc) =>
          acc.id === service
            ? { ...acc, status, lastChecked: Date.now() }
            : acc
        )
      );
      setLoading((prev) => ({ ...prev, [service]: false }));
    }).then((dispose) => {
      unlisten = dispose;
    });

    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  }, [checkStatus]);

  const expiredService = accounts.find((a) => a.status === 'expired')?.id || null;
  const isPausedForAuth = expiredService !== null;

  const ytStatus = accounts.find((a) => a.id === 'ytmusic')?.status || 'disconnected';
  const spotifyStatus = accounts.find((a) => a.id === 'spotify')?.status || 'disconnected';

  return (
    <AuthContext.Provider
      value={{
        accounts,
        loading,
        connect,
        disconnect,
        checkStatus,
        isPausedForAuth,
        expiredService,
        ytStatus,
        spotifyStatus,
      }}
    >
      {children}
    </AuthContext.Provider>
  );
}

export function useAuth() {
  const context = useContext(AuthContext);
  if (!context) {
    throw new Error('useAuth must be used within an AuthProvider');
  }
  return context;
}
