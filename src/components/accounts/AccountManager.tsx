import { useAuthStatus } from '../../hooks/useAuthStatus';
import { AccountCard } from './AccountCard';
import { ShieldCheck, RefreshCw, AlertCircle, Laptop } from 'lucide-react';
import { ServiceId } from '../../types/auth';
import { isTauri } from '../../utils/tauri';

export function AccountManager() {
  const {
    accounts,
    loading,
    connect,
    disconnect,
    checkStatus,
    isPausedForAuth,
    expiredService,
  } = useAuthStatus();

  const inTauri = isTauri();

  return (
    <div className="flex flex-col gap-6 max-w-4xl mx-auto w-full">
      {/* Browser Environment Notice */}
      {!inTauri && (
        <div className="p-4 rounded-lg bg-zinc-900 border border-zinc-700/80 flex items-start gap-3 text-zinc-300">
          <Laptop className="w-5 h-5 text-zinc-400 shrink-0 mt-0.5" />
          <div className="flex-1">
            <h4 className="text-xs font-semibold text-zinc-200">
              Browser Preview Active
            </h4>
            <p className="text-xs text-zinc-400 mt-0.5 leading-relaxed">
              You are viewing the frontend inside an external web browser. Native WebView authentication popups and OS Keyring operations execute exclusively within the <strong>SoundShift desktop application window</strong> running on your desktop. Switch to the desktop app window to connect accounts.
            </p>
          </div>
        </div>
      )}

      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 pb-4 border-b border-zinc-800/80">
        <div>
          <h2 className="text-lg font-semibold text-zinc-100 tracking-tight">
            Streaming Accounts
          </h2>
          <p className="text-xs text-zinc-400 mt-1">
            Connect your streaming services via native isolated sessions. No manual API keys required.
          </p>
        </div>

        <button
          onClick={() => checkStatus()}
          className="self-start sm:self-auto inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md text-zinc-300 bg-zinc-900 border border-zinc-800 hover:bg-zinc-800 hover:text-zinc-100 transition-colors cursor-pointer"
        >
          <RefreshCw className="w-3.5 h-3.5" />
          Refresh Status
        </button>
      </div>

      {/* Mid-Job Pause & Prompt Alert per D-08 */}
      {isPausedForAuth && expiredService && (
        <div className="p-4 rounded-lg bg-amber-950/20 border border-amber-800/60 flex items-start gap-3 text-amber-200">
          <AlertCircle className="w-5 h-5 text-amber-400 shrink-0 mt-0.5" />
          <div className="flex-1">
            <h4 className="text-xs font-semibold text-amber-300">
              Session Expired During Operation
            </h4>
            <p className="text-xs text-amber-200/80 mt-0.5 leading-relaxed">
              Your {expiredService === 'ytmusic' ? 'YouTube Music' : 'Spotify'} authentication has expired. Background transfers are paused until you re-authenticate.
            </p>
            <button
              onClick={() => connect(expiredService as ServiceId)}
              className="mt-2.5 inline-flex items-center gap-1.5 px-3 py-1 text-xs font-medium rounded bg-amber-500/20 text-amber-300 border border-amber-500/30 hover:bg-amber-500/30 transition-colors cursor-pointer"
            >
              Re-authenticate {expiredService === 'ytmusic' ? 'YouTube Music' : 'Spotify'}
            </button>
          </div>
        </div>
      )}

      {/* Account Cards Grid */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        {accounts.map((account) => (
          <AccountCard
            key={account.id}
            account={account}
            isLoading={loading[account.id]}
            onConnect={() => connect(account.id)}
            onDisconnect={() => disconnect(account.id)}
          />
        ))}
      </div>

      {/* Local Storage & Security Guarantee */}
      <div className="flex items-start gap-3 p-4 rounded-lg bg-zinc-950/40 border border-zinc-800/60 text-xs text-zinc-400">
        <ShieldCheck className="w-5 h-5 text-emerald-500/80 shrink-0 mt-0.5" />
        <div className="space-y-1">
          <p className="font-medium text-zinc-300">Local-First Privacy Architecture</p>
          <p className="leading-relaxed text-zinc-500">
            Session tokens are stored exclusively on your device inside the OS Credential Manager (or local AES-256 encrypted SQLite). Disconnecting an account immediately purges credentials while preserving reusable track match caches.
          </p>
        </div>
      </div>
    </div>
  );
}
