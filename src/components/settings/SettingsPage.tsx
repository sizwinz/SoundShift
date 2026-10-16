import { useEffect, useState } from "react";
import {
  Check,
  Database,
  Gauge,
  RotateCcw,
  ShieldCheck,
  SlidersHorizontal,
  Sparkles,
  Trash2,
} from "lucide-react";
import { Card, Button } from "../ui";

const SETTINGS_KEY = "soundshift.settings";

export interface SoundShiftSettings {
  concurrency: number;
  confirmBeforeTransfer: boolean;
  reducedMotion: boolean;
  keepTransferLogs: boolean;
}

export const defaultSettings: SoundShiftSettings = {
  concurrency: 4,
  confirmBeforeTransfer: true,
  reducedMotion: false,
  keepTransferLogs: true,
};

export function readSoundShiftSettings(): SoundShiftSettings {
  try {
    const stored = JSON.parse(localStorage.getItem(SETTINGS_KEY) || "{}") as Partial<SoundShiftSettings>;
    return { ...defaultSettings, ...stored };
  } catch {
    return defaultSettings;
  }
}

export function SettingsPage() {
  const [settings, setSettings] = useState<SoundShiftSettings>(() => readSoundShiftSettings());
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
    document.documentElement.classList.toggle("reduce-motion", settings.reducedMotion);
    setSaved(true);
    const timeout = window.setTimeout(() => setSaved(false), 1200);
    return () => window.clearTimeout(timeout);
  }, [settings]);

  const update = <K extends keyof SoundShiftSettings>(key: K, value: SoundShiftSettings[K]) => {
    setSettings((current) => ({ ...current, [key]: value }));
  };

  const reset = () => {
    setSettings(defaultSettings);
    localStorage.removeItem("soundshift.transfer-options");
  };

  const clearLocalData = () => {
    localStorage.removeItem(SETTINGS_KEY);
    localStorage.removeItem("soundshift.transfer-options");
    setSettings(defaultSettings);
  };

  return (
    <div className="mx-auto max-w-[1120px] pb-16 w-full">
      <div className="mb-6 pb-3 border-b border-white/[0.08] flex items-center justify-between">
        <div>
          <h1 className="text-base font-semibold text-zinc-100 flex items-center gap-2">
            <SlidersHorizontal className="w-4 h-4 text-emerald-400" />
            <span>Settings & Preferences</span>
          </h1>
          <p className="mt-0.5 text-xs text-zinc-400">
            Control transfer concurrency and local client preferences. All configuration is strictly local.
          </p>
        </div>

        {saved && (
          <span className="inline-flex items-center gap-1.5 text-xs text-emerald-400 bg-emerald-950/40 border border-emerald-500/30 px-2.5 py-1 rounded-full animate-in fade-in">
            <Check className="w-3.5 h-3.5" />
            <span>Saved</span>
          </span>
        )}
      </div>

      <div className="grid gap-5 lg:grid-cols-[minmax(0,1fr)_320px]">
        <div className="space-y-5">
          {/* Performance Section */}
          <Card className="overflow-hidden">
            <div className="flex items-start gap-3 border-b border-white/[0.06] px-5 py-4 bg-[#0e0e13]/50">
              <div className="rounded-lg border border-emerald-500/20 bg-emerald-500/10 p-2 text-emerald-400">
                <Gauge className="h-4 w-4" />
              </div>
              <div>
                <h2 className="text-sm font-semibold text-zinc-100">Transfer Performance</h2>
                <p className="mt-0.5 text-xs text-zinc-400">Tune how many provider network requests execute in parallel.</p>
              </div>
            </div>

            <div className="space-y-5 px-5 py-5">
              <label className="block">
                <div className="flex items-center justify-between text-xs">
                  <span className="font-medium text-zinc-200">Concurrent Workers</span>
                  <span className="rounded-md bg-white/[0.06] border border-white/[0.08] px-2.5 py-0.5 font-mono text-emerald-400 tabular-nums">
                    {settings.concurrency} workers
                  </span>
                </div>
                <input
                  type="range"
                  min="1"
                  max="8"
                  value={settings.concurrency}
                  onChange={(event) => update("concurrency", Number(event.target.value))}
                  className="mt-3 h-1.5 w-full cursor-pointer accent-emerald-500"
                  aria-label="Concurrent workers"
                />
                <div className="mt-2 flex justify-between text-[11px] font-mono text-zinc-500">
                  <span>1 (safest)</span>
                  <span>4 (recommended)</span>
                  <span>8 (fastest)</span>
                </div>
              </label>

              <SettingToggle
                title="Pre-transfer confirmation modal"
                description="Show the target playlist destination selector and batch review screen before dispatching tasks."
                checked={settings.confirmBeforeTransfer}
                onChange={(checked) => update("confirmBeforeTransfer", checked)}
              />

              <SettingToggle
                title="Preserve live transfer logs"
                description="Keep streaming log events in memory during transfers to allow detailed console telemetry inspection."
                checked={settings.keepTransferLogs}
                onChange={(checked) => update("keepTransferLogs", checked)}
              />
            </div>
          </Card>

          {/* Interface Section */}
          <Card className="overflow-hidden">
            <div className="flex items-start gap-3 border-b border-white/[0.06] px-5 py-4 bg-[#0e0e13]/50">
              <div className="rounded-lg border border-white/[0.1] bg-white/[0.04] p-2 text-zinc-300">
                <Sparkles className="h-4 w-4" />
              </div>
              <div>
                <h2 className="text-sm font-semibold text-zinc-100">User Interface & Motion</h2>
                <p className="mt-0.5 text-xs text-zinc-400">Accessibility and desktop animation preferences.</p>
              </div>
            </div>

            <div className="space-y-4 px-5 py-5">
              <SettingToggle
                title="Reduce motion"
                description="Disable sliding drawer transitions and pulse animations for high performance or motion sensitivity."
                checked={settings.reducedMotion}
                onChange={(checked) => update("reducedMotion", checked)}
              />
            </div>
          </Card>

          {/* Storage Actions */}
          <Card className="overflow-hidden">
            <div className="flex items-start gap-3 border-b border-white/[0.06] px-5 py-4 bg-[#0e0e13]/50">
              <div className="rounded-lg border border-white/[0.1] bg-white/[0.04] p-2 text-zinc-300">
                <Database className="h-4 w-4" />
              </div>
              <div>
                <h2 className="text-sm font-semibold text-zinc-100">Storage & Reset</h2>
                <p className="mt-0.5 text-xs text-zinc-400">Manage client cache and reset local preferences.</p>
              </div>
            </div>

            <div className="flex flex-col gap-3 px-5 py-5 sm:flex-row sm:items-center sm:justify-between">
              <div>
                <div className="text-xs font-semibold text-zinc-200">Reset default preferences</div>
                <p className="mt-0.5 text-xs text-zinc-400">Restore worker concurrency and interface options to initial defaults.</p>
              </div>
              <Button
                variant="secondary"
                size="sm"
                onClick={reset}
              >
                <RotateCcw className="h-3.5 w-3.5" />
                <span>Reset Defaults</span>
              </Button>
            </div>

            <div className="border-t border-white/[0.06] px-5 py-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              <div>
                <div className="text-xs font-semibold text-rose-300">Clear cached transfer preferences</div>
                <p className="mt-0.5 text-xs text-zinc-400">Clears locally memorized playlist targets without modifying OS credentials or snapshots.</p>
              </div>
              <Button
                variant="danger"
                size="sm"
                onClick={clearLocalData}
              >
                <Trash2 className="h-3.5 w-3.5" />
                <span>Clear Cache</span>
              </Button>
            </div>
          </Card>
        </div>

        {/* Sidebar Info Card */}
        <div className="space-y-4">
          <Card className="p-5 space-y-3 bg-[#0d0d12]">
            <div className="flex items-center gap-2 text-emerald-400">
              <ShieldCheck className="h-4 w-4" />
              <span className="text-xs font-semibold">Local-First Architecture</span>
            </div>
            <p className="text-xs text-zinc-400 leading-relaxed">
              SoundShift does not use remote analytics, external telemetry servers, or cloud databases. All session keys are anchored to your local OS keychain.
            </p>
          </Card>

          <Card className="p-5 space-y-3 font-mono text-xs">
            <span className="text-[10px] uppercase tracking-wider text-zinc-500 font-semibold font-sans">
              System Specifications
            </span>
            <div className="space-y-1.5 text-zinc-300 tabular-nums">
              <div className="flex justify-between">
                <span className="text-zinc-500 font-sans">Client Core:</span>
                <span>Tauri v2 + Rust</span>
              </div>
              <div className="flex justify-between">
                <span className="text-zinc-500 font-sans">Database:</span>
                <span>Embedded SQLite (WAL)</span>
              </div>
              <div className="flex justify-between">
                <span className="text-zinc-500 font-sans">Typography:</span>
                <span>Inter (Offline TTF)</span>
              </div>
              <div className="flex justify-between">
                <span className="text-zinc-500 font-sans">Version:</span>
                <span>0.9.0-beta</span>
              </div>
            </div>
          </Card>
        </div>
      </div>
    </div>
  );
}

function SettingToggle({
  title,
  description,
  checked,
  onChange,
}: {
  title: string;
  description: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="flex items-start justify-between gap-4 cursor-pointer py-1">
      <div className="min-w-0 flex-1">
        <div className="text-xs font-semibold text-zinc-200">{title}</div>
        <div className="mt-0.5 text-xs text-zinc-400 leading-relaxed">{description}</div>
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        onClick={() => onChange(!checked)}
        className={`relative inline-flex h-5 w-9 shrink-0 rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none cursor-pointer mt-0.5 ${
          checked ? "bg-emerald-500" : "bg-white/[0.12]"
        }`}
      >
        <span
          className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow-lg ring-0 transition duration-200 ease-in-out ${
            checked ? "translate-x-4 bg-zinc-950" : "translate-x-0 bg-zinc-300"
          }`}
        />
      </button>
    </label>
  );
}
