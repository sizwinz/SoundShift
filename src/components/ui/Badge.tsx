import React from "react";

export type BadgeVariant =
  | "exact"
  | "amber"
  | "unmatched"
  | "duplicate"
  | "neutral"
  | "connected"
  | "disconnected"
  | "expired";

interface BadgeProps {
  variant: BadgeVariant;
  children: React.ReactNode;
  icon?: React.ReactNode;
  className?: string;
  count?: number;
}

export const Badge: React.FC<BadgeProps> = ({
  variant,
  children,
  icon,
  className = "",
  count,
}) => {
  const variantStyles: Record<BadgeVariant, string> = {
    exact: "bg-emerald-950/60 text-emerald-400 border-emerald-500/30",
    amber: "bg-amber-950/60 text-amber-400 border-amber-500/30",
    unmatched: "bg-rose-950/60 text-rose-400 border-rose-500/30",
    duplicate: "bg-zinc-900 text-zinc-400 border-zinc-700/60",
    neutral: "bg-zinc-900/80 text-zinc-300 border-white/10",
    connected: "bg-emerald-950/50 text-emerald-300 border-emerald-500/30",
    disconnected: "bg-zinc-900/60 text-zinc-500 border-zinc-800",
    expired: "bg-amber-950/60 text-amber-300 border-amber-500/40",
  };

  return (
    <span
      className={`inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-[11px] font-medium border ${variantStyles[variant]} ${className}`}
    >
      {icon}
      <span>{children}</span>
      {count !== undefined && (
        <span className="font-mono text-[10px] opacity-80 tabular-nums">
          ({count})
        </span>
      )}
    </span>
  );
};
