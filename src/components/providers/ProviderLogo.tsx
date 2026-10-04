import React from "react";

interface ProviderLogoProps {
  provider: "spotify" | "ytmusic";
  size?: "sm" | "md" | "lg";
  className?: string;
}

const providerAssets = {
  spotify: {
    src: "/spotify-logo.webp",
    alt: "Spotify",
  },
  ytmusic: {
    src: "/youtube-music-logo.webp",
    alt: "YouTube Music",
  },
} as const;

export const ProviderLogo: React.FC<ProviderLogoProps> = ({
  provider,
  size = "md",
  className = "",
}) => {
  const asset = providerAssets[provider];
  const sizeClass = {
    sm: "h-4 w-4",
    md: "h-5 w-5",
    lg: "h-7 w-7",
  }[size];

  return (
    <img
      src={asset.src}
      alt={asset.alt}
      className={`shrink-0 object-contain ${sizeClass} ${className}`}
    />
  );
};
