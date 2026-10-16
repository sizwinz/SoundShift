import { ServiceAccount } from '../../types/auth';
import { CheckCircle2, AlertTriangle, LogIn, LogOut, RefreshCw } from 'lucide-react';
import { ProviderLogo } from '../providers/ProviderLogo';
import { Card, Button, Badge } from '../ui';

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
    <Card className="p-5 flex flex-col justify-between hover:border-white/[0.16] transition-colors">
      <div>
        <div className="flex items-center justify-between gap-3 mb-3.5">
          <div className="flex items-center gap-3">
            <div className="flex items-center justify-center w-10 h-10 rounded-xl bg-white/[0.04] border border-white/[0.08]">
              <ProviderLogo provider={account.id} size="md" />
            </div>
            <div>
              <h3 className="text-sm font-semibold text-zinc-100">
                {account.name}
              </h3>
              <p className="text-[11px] font-mono text-zinc-500 uppercase tracking-wider">{account.id}</p>
            </div>
          </div>

          <div>
            {isConnected && (
              <Badge variant="connected" icon={<CheckCircle2 className="w-3 h-3 text-emerald-400" />}>
                Connected
              </Badge>
            )}
            {isExpired && (
              <Badge variant="expired" icon={<AlertTriangle className="w-3 h-3 text-amber-400" />}>
                Expired
              </Badge>
            )}
            {!isConnected && !isExpired && (
              <Badge variant="disconnected">
                Disconnected
              </Badge>
            )}
          </div>
        </div>

        <p className="text-xs text-zinc-400 leading-relaxed mb-4">
          {account.description}
        </p>
      </div>

      <div className="pt-3.5 border-t border-white/[0.06] flex items-center justify-between">
        <span className="text-[11px] text-zinc-500 font-mono">
          {account.lastChecked
            ? `Checked ${new Date(account.lastChecked).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`
            : 'Zero manual API keys'}
        </span>

        <div className="flex items-center gap-2">
          {isConnected ? (
            <Button
              variant="danger"
              size="sm"
              onClick={onDisconnect}
              disabled={isLoading}
            >
              {isLoading ? (
                <RefreshCw className="w-3.5 h-3.5 animate-spin" />
              ) : (
                <LogOut className="w-3.5 h-3.5" />
              )}
              <span>Disconnect</span>
            </Button>
          ) : isExpired ? (
            <Button
              variant="amber"
              size="sm"
              onClick={onConnect}
              disabled={isLoading}
            >
              <RefreshCw className={`w-3.5 h-3.5 ${isLoading ? "animate-spin" : ""}`} />
              <span>Reconnect</span>
            </Button>
          ) : (
            <Button
              variant="primary"
              size="sm"
              onClick={onConnect}
              disabled={isLoading}
            >
              {isLoading ? (
                <RefreshCw className="w-3.5 h-3.5 animate-spin" />
              ) : (
                <LogIn className="w-3.5 h-3.5" />
              )}
              <span>Connect</span>
            </Button>
          )}
        </div>
      </div>
    </Card>
  );
}
