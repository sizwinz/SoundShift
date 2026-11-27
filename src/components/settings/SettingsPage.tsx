import { useEffect, useId, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
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
import { isTauri } from "../../utils/tauri";

const SETTINGS_KEY = "soundshift.settings";

export interface SoundShiftSettings {
  concurrency: number;
  matchingConcurrency: number;
  confirmBeforeTransfer: boolean;
  reducedMotion: boolean;
  keepTransferLogs: boolean;
  minimizeToTray: boolean;
}

export const defaultSettings: SoundShiftSettings = {
  concurrency: 1,
  matchingConcurrency: 4,
  confirmBeforeTransfer: true,
  reducedMotion: false,
  keepTransferLogs: true,
  minimizeToTray: false,
};

export function readSoundShiftSettings(): SoundShiftSettings {
  try {
    const stored = JSON.parse(localStorage.getItem(SETTINGS_KEY) || "{}") as Partial<SoundShiftSettings>;
    const rawMatching = typeof stored.matchingConcurrency === "number" ? stored.matchingConcurrency : defaultSettings.matchingConcurrency;
    return {
      ...defaultSettings,
      ...stored,
      concurrency: 1,
      matchingConcurrency: Math.min(4, Math.max(1, Math.round(rawMatching))),
    };
  } catch {
    return defaultSettings;
  }
}

export function SettingsPage() {
  const [settings, setSettings] = useState<SoundShiftSettings>(() => readSoundShiftSettings());
  const [saved, setSaved] = useState(false);
  const [cacheNotice, setCacheNotice] = useState<string | null>(null);

  useEffect(() => {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
    document.documentElement.classList.toggle("reduce-motion", settings.reducedMotion);
    setSaved(true);
    const timeout = window.setTimeout(() => setSaved(false), 1200);
    return () => window.clearTimeout(timeout);
  }, [settings]);

  const update = <K extends keyof SoundShiftSettings>(key: K, value: SoundShiftSettings[K]) => {
    setSettings((current) => ({ ...current, [key]: value }));
    if (key === "minimizeToTray" && isTauri()) {
      void invoke("set_tray_preference", { enabled: Boolean(value) }).catch(() => {});
    }
  };

  const reset = () => {
    setSettings(defaultSettings);
    localStorage.removeItem("soundshift.transfer-options");
    if (isTauri()) {
      void invoke("set_tray_preference", { enabled: false }).catch(() => {});
    }
  };

  const clearLocalData = () => {
    localStorage.removeItem(SETTINGS_KEY);
    localStorage.removeItem("soundshift.transfer-options");
    sessionStorage.removeItem("soundshift.staged-session");
    setSettings(defaultSettings);
    setCacheNotice("Cleared local UI and target playlist preferences.");
  };

  const clearMatchCache = async () => {
    if (!isTauri()) {
      setCacheNotice("Track match cache resets inside the desktop app.");
      return;
    }
    try {
      const removed = await invoke<number>("clear_match_cache");
      setCacheNotice(`Cleared ${removed} cached track match entries.`);
    } catch (err) {
      setCacheNotice(`Failed to clear match cache: ${err instanceof Error ? err.message : String(err)}`);
    }
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
            Configure local client preferences. Playlist writes use one writer to preserve order.
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
                <h2 className="text-sm font-semibold text-zinc-100">Transfer & Matching Performance</h2>
                <p className="mt-0.5 text-xs text-zinc-400">Destination writes run sequentially; catalog matching runs concurrently.</p>
              </div>
            </div>

            <div className="space-y-5 px-5 py-5">
              <div>
                <div className="flex items-center justify-between text-xs">
                  <span className="font-medium text-zinc-200">Concurrent Writers</span>
                  <span className="rounded-md bg-white/[0.06] border border-white/[0.08] px-2.5 py-0.5 font-mono text-emerald-400 tabular-nums">
                    1 writer
                  </span>
                </div>
                <input
                  type="range"
                  min="1"
                  max="1"
                  value={1}
                  disabled
                  readOnly
                  className="mt-3 h-1.5 w-full cursor-not-allowed accent-emerald-500 opacity-60"
                  aria-label="Concurrent workers"
                />
                <div className="mt-1.5 flex justify-between text-[11px] font-mono text-zinc-500">
                  <span>One ordered playlist writer enforced for safety</span>
                </div>
              </div>

              <div className="border-t border-white/[0.06] pt-4">
                <div className="flex items-center justify-between text-xs">
                  <label htmlFor="matching-concurrency-range" className="font-medium text-zinc-200">
                    Matching Search Workers
                  </label>
                  <span className="rounded-md bg-white/[0.06] border border-white/[0.08] px-2.5 py-0.5 font-mono text-emerald-400 tabular-nums">
                    {settings.matchingConcurrency} {settings.matchingConcurrency === 1 ? "worker" : "workers"}
                  </span>
                </div>
                <input
                  id="matching-concurrency-range"
                  type="range"
                  min={1}
                  max={4}
                  step={1}
                  value={settings.matchingConcurrency}
                  onChange={(e) => update("matchingConcurrency", Number(e.target.value))}
                  className="mt-3 h-1.5 w-full cursor-pointer accent-emerald-500"
                  aria-label="Matching search workers"
                />
                <div className="mt-1.5 flex justify-between text-[11px] font-mono text-zinc-500">
                  <span>1 (gentle on rate limits)</span>
                  <span>4 (fastest matching)</span>
                </div>
              </div>

              <SettingToggle
                title="Pre-transfer confirmation modal"
                description="Show the target playlist destination selector and batch review modal before dispatching transfers."
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
                <h2 className="text-sm font-semibold text-zinc-100">Desktop Shell & Motion</h2>
                <p className="mt-0.5 text-xs text-zinc-400">Accessibility, window close, and desktop animation preferences.</p>
              </div>
            </div>

            <div className="space-y-4 px-5 py-5">
              <SettingToggle
                title="Reduce motion"
                description="Disable sliding drawer transitions and pulse animations for high performance or motion sensitivity."
                checked={settings.reducedMotion}
                onChange={(checked) => update("reducedMotion", checked)}
              />

              <SettingToggle
                title="Minimize to system tray on close"
                description="Keep SoundShift in the system tray when closing the window so hourly playlist sync checks continue running."
                checked={settings.minimizeToTray}
                onChange={(checked) => update("minimizeToTray", checked)}
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
                <p className="mt-0.5 text-xs text-zinc-400">Manage cached matches and reset local preferences.</p>
              </div>
            </div>

            {cacheNotice && (
              <div className="mx-5 mt-4 rounded-lg border border-emerald-500/30 bg-emerald-950/30 px-3 py-2 text-xs text-emerald-300">
                {cacheNotice}
              </div>
            )}

            <div className="flex flex-col gap-3 px-5 py-5 sm:flex-row sm:items-center sm:justify-between">
              <div>
                <div className="text-xs font-semibold text-zinc-200">Reset default preferences</div>
                <p className="mt-0.5 text-xs text-zinc-400">Restore interface options to initial defaults.</p>
              </div>
              <Button variant="secondary" size="sm" onClick={reset}>
                <RotateCcw className="h-3.5 w-3.5" />
                <span>Reset Defaults</span>
              </Button>
            </div>

            <div className="border-t border-white/[0.06] px-5 py-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              <div>
                <div className="text-xs font-semibold text-zinc-200">Clear track match cache</div>
                <p className="mt-0.5 text-xs text-zinc-400">Removes saved ISRC, fuzzy, and manual match overrides so future runs re-search fresh targets.</p>
              </div>
              <Button variant="secondary" size="sm" onClick={clearMatchCache}>
                <Trash2 className="h-3.5 w-3.5" />
                <span>Clear Match Cache</span>
              </Button>
            </div>

            <div className="border-t border-white/[0.06] px-5 py-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              <div>
                <div className="text-xs font-semibold text-rose-300">Clear cached UI preferences</div>
                <p className="mt-0.5 text-xs text-zinc-400">Clears locally memorized targets and draft staging state without modifying OS credentials or snapshots.</p>
              </div>
              <Button variant="danger" size="sm" onClick={clearLocalData}>
                <Trash2 className="h-3.5 w-3.5" />
                <span>Clear UI Cache</span>
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
                <span>0.10.0</span>
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
  const switchId = useId();
  const descId = useId();

  return (
    <div className="flex items-start justify-between gap-4 py-1">
      <div className="min-w-0 flex-1">
        <label
          htmlFor={switchId}
          className="text-xs font-semibold text-zinc-200 cursor-pointer block"
        >
          {title}
        </label>
        <p id={descId} className="mt-0.5 text-xs text-zinc-400 leading-relaxed">
          {description}
        </p>
      </div>
      <button
        id={switchId}
        type="button"
        role="switch"
        aria-checked={checked}
        aria-describedby={descId}
        onClick={() => onChange(!checked)}
        className={`relative inline-flex h-5 w-9 shrink-0 rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-emerald-400 cursor-pointer mt-0.5 ${
          checked ? "bg-emerald-500" : "bg-white/[0.12]"
        }`}
      >
        <span
          className={`pointer-events-none inline-block h-4 w-4 transform rounded-full shadow-lg ring-0 transition duration-200 ease-in-out ${
            checked ? "translate-x-4 bg-zinc-950" : "translate-x-0 bg-zinc-300"
          }`}
        />
      </button>
    </div>
  );
}
