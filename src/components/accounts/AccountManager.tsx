import { useAuthStatus } from '../../hooks/useAuthStatus';
import { AccountCard } from './AccountCard';
import { ShieldCheck, RefreshCw, AlertCircle, Laptop } from 'lucide-react';
import { ServiceId } from '../../types/auth';
import { isTauri } from '../../utils/tauri';
import { Button, Card } from '../ui';

interface AccountManagerProps {
  isModal?: boolean;
}

export function AccountManager({ isModal = false }: AccountManagerProps) {
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
    <div className={`flex flex-col gap-5 ${isModal ? "w-full" : "max-w-4xl mx-auto w-full"}`}>
      {/* Browser Environment Notice */}
      {!inTauri && (
        <div className="p-3.5 rounded-xl bg-zinc-900/80 border border-zinc-700/60 flex items-start gap-3 text-zinc-300">
          <Laptop className="w-5 h-5 text-zinc-400 shrink-0 mt-0.5" />
          <div className="flex-1">
            <h4 className="text-xs font-semibold text-zinc-200">
              Browser Preview Active
            </h4>
            <p className="text-xs text-zinc-400 mt-0.5 leading-relaxed">
              You are viewing the frontend inside an external web browser. Native WebView authentication popups and OS Keyring operations execute exclusively within the <strong>SoundShift desktop application window</strong>.
            </p>
          </div>
        </div>
      )}

      {/* Header if not in modal */}
      {!isModal && (
        <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 pb-4 border-b border-white/[0.08]">
          <div>
            <h2 className="text-base font-semibold text-zinc-100">
              Connected Streaming Services
            </h2>
            <p className="text-xs text-zinc-400 mt-0.5">
              Isolated WebView sessions with zero user-supplied API keys or credentials.
            </p>
          </div>

          <Button
            variant="secondary"
            size="sm"
            onClick={() => checkStatus()}
          >
            <RefreshCw className="w-3.5 h-3.5" />
            <span>Refresh Status</span>
          </Button>
        </div>
      )}

      {/* Mid-Job Pause & Prompt Alert */}
      {isPausedForAuth && expiredService && (
        <div className="p-4 rounded-xl bg-amber-950/20 border border-amber-800/60 flex items-start gap-3 text-amber-200">
          <AlertCircle className="w-5 h-5 text-amber-400 shrink-0 mt-0.5" />
          <div className="flex-1">
            <h4 className="text-xs font-semibold text-amber-300">
              Session Expired During Operation
            </h4>
            <p className="text-xs text-amber-200/80 mt-0.5 leading-relaxed">
              Your {expiredService === 'ytmusic' ? 'YouTube Music' : 'Spotify'} authentication has expired. Background transfers are paused until you re-authenticate.
            </p>
            <Button
              variant="amber"
              size="xs"
              onClick={() => connect(expiredService as ServiceId)}
              className="mt-2.5"
            >
              Re-authenticate {expiredService === 'ytmusic' ? 'YouTube Music' : 'Spotify'}
            </Button>
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
      <Card className="flex items-start gap-3 p-4 text-xs text-zinc-400">
        <ShieldCheck className="w-5 h-5 text-emerald-400 shrink-0 mt-0.5" />
        <div className="space-y-0.5">
          <p className="font-semibold text-zinc-200">Local-First Privacy Architecture</p>
          <p className="leading-relaxed text-zinc-400 text-[11px]">
            Session tokens are stored exclusively on your device inside the OS Credential Manager (or local AES-256 encrypted SQLite). Disconnecting an account immediately purges credentials while preserving reusable track match caches.
          </p>
        </div>
      </Card>
    </div>
  );
}
