import React, { HTMLAttributes } from "react";

interface CardProps extends HTMLAttributes<HTMLDivElement> {
  elevated?: boolean;
  interactive?: boolean;
}

export const Card: React.FC<CardProps> = ({
  elevated = false,
  interactive = false,
  children,
  className = "",
  ...props
}) => {
  return (
    <div
      className={`rounded-xl border border-white/[0.08] transition-all duration-150 ${
        elevated
          ? "bg-[#0e0e13] shadow-lg shadow-black/30"
          : "bg-[#0a0a0e]"
      } ${
        interactive
          ? "hover:border-white/[0.18] hover:bg-[#121218] cursor-pointer"
          : ""
      } ${className}`}
      {...props}
    >
      {children}
    </div>
  );
};
