import { useEffect, useState } from "react";
import type { ReactNode } from "react";
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
    <div className="mx-auto max-w-[1120px] pb-12">
      <div className="mb-6">
        <h1 className="text-lg font-bold text-zinc-100">Settings</h1>
        <p className="mt-1 text-xs text-zinc-500">
          Control transfer behavior and local app preferences. Nothing leaves this device.
        </p>
      </div>

      <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_300px]">
        <div className="space-y-4">
          <section className="rounded-xl border border-[#27272a] bg-[#09090b]">
            <div className="flex items-start gap-3 border-b border-[#27272a] px-5 py-4">
              <div className="rounded-lg border border-emerald-500/20 bg-emerald-500/10 p-2 text-emerald-400">
                <Gauge className="h-4 w-4" />
              </div>
              <div>
                <h2 className="text-sm font-semibold text-zinc-100">Transfer performance</h2>
                <p className="mt-1 text-xs text-zinc-500">Tune how many provider requests can run at once.</p>
              </div>
            </div>
            <div className="space-y-5 px-5 py-5">
              <label className="block">
                <div className="flex items-center justify-between text-xs">
                  <span className="font-medium text-zinc-200">Concurrent workers</span>
                  <span className="rounded bg-zinc-800 px-2 py-1 font-mono text-emerald-400">{settings.concurrency}</span>
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
                <div className="mt-2 flex justify-between text-[10px] text-zinc-600">
                  <span>Safer, slower</span><span>1</span><span>8</span><span>Faster</span>
                </div>
                <p className="mt-2 text-[11px] text-zinc-500">
                  Note: YouTube Music destinations automatically serialize to 1 worker to prevent 409 Conflict collisions.
                </p>
              </label>
              <SettingToggle
                icon={<ShieldCheck className="h-4 w-4" />}
                title="Confirm before every transfer"
                description="Keep the destination and snapshot review step enabled."
                checked={settings.confirmBeforeTransfer}
                onChange={(value) => update("confirmBeforeTransfer", value)}
              />
            </div>
          </section>

          <section className="rounded-xl border border-[#27272a] bg-[#09090b]">
            <div className="flex items-start gap-3 border-b border-[#27272a] px-5 py-4">
              <div className="rounded-lg border border-sky-500/20 bg-sky-500/10 p-2 text-sky-400">
                <SlidersHorizontal className="h-4 w-4" />
              </div>
              <div>
                <h2 className="text-sm font-semibold text-zinc-100">Interface</h2>
                <p className="mt-1 text-xs text-zinc-500">Adjust feedback and motion without changing transfer results.</p>
              </div>
            </div>
            <div className="space-y-1 px-5 py-4">
              <SettingToggle
                icon={<Sparkles className="h-4 w-4" />}
                title="Reduce interface motion"
                description="Use fewer animated transitions and status effects."
                checked={settings.reducedMotion}
                onChange={(value) => update("reducedMotion", value)}
              />
              <SettingToggle
                icon={<Database className="h-4 w-4" />}
                title="Keep transfer logs"
                description="Retain recent transfer activity in the local session."
                checked={settings.keepTransferLogs}
                onChange={(value) => update("keepTransferLogs", value)}
              />
            </div>
          </section>
        </div>

        <aside className="h-fit rounded-xl border border-[#27272a] bg-[#09090b] p-5">
          <div className="flex items-center gap-2 text-sm font-semibold text-zinc-100">
            <ShieldCheck className="h-4 w-4 text-emerald-400" />
            Local-first privacy
          </div>
          <p className="mt-3 text-xs leading-relaxed text-zinc-500">
            Preferences, credentials, snapshots, and transfer history stay on this machine. SoundShift does not use telemetry.
          </p>
          <div className="mt-5 space-y-2 border-t border-[#27272a] pt-4 text-[11px] text-zinc-500">
            <div className="flex justify-between"><span>Storage</span><span className="text-zinc-300">Local</span></div>
            <div className="flex justify-between"><span>Workers</span><span className="text-zinc-300">1 to 8</span></div>
            <div className="flex justify-between"><span>Network</span><span className="text-zinc-300">Provider only</span></div>
          </div>
          <div className="mt-5 space-y-2">
            <button type="button" onClick={reset} className="flex w-full items-center justify-center gap-2 rounded-md border border-zinc-800 px-3 py-2 text-xs text-zinc-300 transition hover:border-zinc-700 hover:bg-zinc-900">
              <RotateCcw className="h-3.5 w-3.5" /> Reset preferences
            </button>
            <button type="button" onClick={clearLocalData} className="flex w-full items-center justify-center gap-2 rounded-md border border-rose-500/20 px-3 py-2 text-xs text-rose-300 transition hover:bg-rose-500/10">
              <Trash2 className="h-3.5 w-3.5" /> Clear local preferences
            </button>
          </div>
          <div className={`mt-4 flex items-center justify-center gap-1.5 text-[11px] text-emerald-400 transition-opacity ${saved ? "opacity-100" : "opacity-0"}`}>
            <Check className="h-3.5 w-3.5" /> Saved locally
          </div>
        </aside>
      </div>
    </div>
  );
}

function SettingToggle({
  icon,
  title,
  description,
  checked,
  onChange,
}: {
  icon: ReactNode;
  title: string;
  description: string;
  checked: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <label className="flex cursor-pointer items-center justify-between gap-4 rounded-lg px-2 py-3 transition hover:bg-zinc-900/70">
      <span className="flex min-w-0 items-start gap-3">
        <span className="mt-0.5 text-zinc-500">{icon}</span>
        <span className="min-w-0">
          <span className="block text-xs font-medium text-zinc-200">{title}</span>
          <span className="mt-1 block text-[11px] leading-relaxed text-zinc-500">{description}</span>
        </span>
      </span>
      <input type="checkbox" checked={checked} onChange={(event) => onChange(event.target.checked)} className="h-4 w-4 shrink-0 accent-emerald-500" />
    </label>
  );
}
