import { ButtonHTMLAttributes, forwardRef } from "react";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary" | "ghost" | "danger" | "amber" | "outline";
  size?: "xs" | "sm" | "md";
  isLoading?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  (
    {
      variant = "secondary",
      size = "sm",
      isLoading,
      children,
      className = "",
      disabled,
      ...props
    },
    ref
  ) => {
    const baseClasses =
      "inline-flex items-center justify-center font-medium rounded-lg transition-all duration-150 cursor-pointer disabled:cursor-not-allowed disabled:opacity-40 select-none";

    const sizeClasses = {
      xs: "text-[11px] px-2.5 py-1 gap-1.5 h-7",
      sm: "text-xs px-3 py-1.5 gap-2 h-8",
      md: "text-xs px-4 py-2 gap-2 h-9",
    }[size];

    const variantClasses = {
      primary:
        "bg-emerald-500 hover:bg-emerald-400 text-zinc-950 font-semibold shadow-[0_2px_12px_rgba(16,185,129,0.22)] active:translate-y-px",
      secondary:
        "bg-[#131318] hover:bg-[#1c1c24] text-zinc-200 border border-white/[0.08] hover:border-white/[0.16] shadow-xs active:translate-y-px",
      ghost:
        "bg-transparent hover:bg-white/[0.06] text-zinc-400 hover:text-zinc-100",
      danger:
        "bg-rose-950/40 hover:bg-rose-900/50 text-rose-300 border border-rose-500/30",
      amber:
        "bg-amber-950/40 hover:bg-amber-900/50 text-amber-300 border border-amber-500/30",
      outline:
        "bg-transparent hover:bg-white/[0.04] text-zinc-300 border border-white/[0.12] hover:border-white/[0.24]",
    }[variant];

    return (
      <button
        ref={ref}
        disabled={disabled || isLoading}
        className={`${baseClasses} ${sizeClasses} ${variantClasses} ${className}`}
        {...props}
      >
        {children}
      </button>
    );
  }
);

Button.displayName = "Button";
