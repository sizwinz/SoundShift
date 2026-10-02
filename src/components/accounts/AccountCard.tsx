import { ServiceAccount } from '../../types/auth';
import { CheckCircle2, AlertTriangle, Radio, LogIn, LogOut, RefreshCw } from 'lucide-react';

interface AccountCardProps {
  account: ServiceAccount;
  isLoading: boolean;
  onConnect: () => void;
  onDisconnect: () => void;
}

export function AccountCard({
  account,
  isLoading,
  onConnect,
  onDisconnect,
}: AccountCardProps) {
  const isConnected = account.status === 'connected';
  const isExpired = account.status === 'expired';

  return (
    <div className="flex flex-col justify-between p-5 rounded-lg border border-zinc-800 bg-zinc-950/70 hover:border-zinc-700 transition-colors">
      <div>
        <div className="flex items-center justify-between gap-3 mb-3">
          <div className="flex items-center gap-2.5">
            <div className="flex items-center justify-center w-9 h-9 rounded-md bg-zinc-900 border border-zinc-800 text-zinc-200">
              <Radio className="w-5 h-5" />
            </div>
            <div>
              <h3 className="text-sm font-semibold text-zinc-100 tracking-tight">
                {account.name}
              </h3>
              <p className="text-xs text-zinc-400">{account.id.toUpperCase()}</p>
            </div>
          </div>

          <div>
            {isConnected && (
              <span className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium bg-emerald-500/10 text-emerald-400 border border-emerald-500/20">
                <CheckCircle2 className="w-3.5 h-3.5" />
                Active
              </span>
            )}
            {isExpired && (
              <span className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium bg-amber-500/10 text-amber-400 border border-amber-500/20">
                <AlertTriangle className="w-3.5 h-3.5" />
                Expired
              </span>
            )}
            {!isConnected && !isExpired && (
              <span className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium bg-zinc-800/80 text-zinc-400 border border-zinc-700/60">
                Disconnected
              </span>
            )}
          </div>
        </div>

        <p className="text-xs text-zinc-400 leading-relaxed mb-4">
          {account.description}
        </p>
      </div>

      <div className="pt-3 border-t border-zinc-800/80 flex items-center justify-between">
        <span className="text-[11px] text-zinc-500">
          {account.lastChecked
            ? `Checked ${new Date(account.lastChecked).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`
            : 'Zero manual API keys'}
        </span>

        <div className="flex items-center gap-2">
          {isConnected ? (
            <button
              onClick={onDisconnect}
              disabled={isLoading}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md text-zinc-400 bg-zinc-900 border border-zinc-800 hover:text-red-400 hover:border-red-900/50 hover:bg-red-950/20 transition-colors disabled:opacity-50 cursor-pointer"
            >
              {isLoading ? (
                <RefreshCw className="w-3.5 h-3.5 animate-spin" />
              ) : (
                <LogOut className="w-3.5 h-3.5" />
              )}
              Disconnect
            </button>
          ) : isExpired ? (
            <button
              onClick={onConnect}
              disabled={isLoading}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md text-amber-300 bg-amber-950/40 border border-amber-800 hover:bg-amber-900/50 transition-colors disabled:opacity-50 cursor-pointer"
            >
              {isLoading ? (
                <RefreshCw className="w-3.5 h-3.5 animate-spin" />
              ) : (
                <RefreshCw className="w-3.5 h-3.5" />
              )}
              Reconnect
            </button>
          ) : (
            <button
              onClick={onConnect}
              disabled={isLoading}
              className="inline-flex items-center gap-1.5 px-3.5 py-1.5 text-xs font-medium rounded-md text-zinc-100 bg-zinc-800 border border-zinc-700 hover:bg-zinc-700 hover:border-zinc-600 transition-colors disabled:opacity-50 cursor-pointer"
            >
              {isLoading ? (
                <RefreshCw className="w-3.5 h-3.5 animate-spin" />
              ) : (
                <LogIn className="w-3.5 h-3.5" />
              )}
              Connect
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
