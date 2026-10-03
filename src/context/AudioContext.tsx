import React, { createContext, useContext, useState, useRef, useEffect, useCallback } from "react";

interface AudioContextType {
  activeTrackId: string | null;
  isPlaying: boolean;
  progress: number;
  duration: number;
  currentTime: number;
  error: string | null;
  playPreview: (trackId: string, previewUrl: string) => void;
  pausePreview: () => void;
  stopPreview: () => void;
}

const AudioContext = createContext<AudioContextType | null>(null);

export const AudioProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [activeTrackId, setActiveTrackId] = useState<string | null>(null);
  const [isPlaying, setIsPlaying] = useState<boolean>(false);
  const [progress, setProgress] = useState<number>(0);
  const [duration, setDuration] = useState<number>(0);
  const [currentTime, setCurrentTime] = useState<number>(0);
  const [error, setError] = useState<string | null>(null);

  // Singleton HTMLAudioElement instance per D-08, D-09
  const audioRef = useRef<HTMLAudioElement | null>(null);

  useEffect(() => {
    const audio = new Audio();
    audioRef.current = audio;

    const handleTimeUpdate = () => {
      if (audio.duration && !isNaN(audio.duration)) {
        setCurrentTime(audio.currentTime);
        setDuration(audio.duration);
        setProgress(audio.currentTime / audio.duration);
      }
    };

    const handleEnded = () => {
      setIsPlaying(false);
      setProgress(0);
      setCurrentTime(0);
      setActiveTrackId(null);
    };

    const handleError = () => {
      setError("Failed to load audio preview stream.");
      setIsPlaying(false);
      setActiveTrackId(null);
    };

    audio.addEventListener("timeupdate", handleTimeUpdate);
    audio.addEventListener("ended", handleEnded);
    audio.addEventListener("error", handleError);

    return () => {
      audio.pause();
      audio.removeEventListener("timeupdate", handleTimeUpdate);
      audio.removeEventListener("ended", handleEnded);
      audio.removeEventListener("error", handleError);
    };
  }, []);

  const playPreview = useCallback((trackId: string, previewUrl: string) => {
    const audio = audioRef.current;
    if (!audio) return;

    setError(null);

    // If already playing this track, toggle pause
    if (activeTrackId === trackId && isPlaying) {
      audio.pause();
      setIsPlaying(false);
      return;
    }

    // Single-active playback concurrency guarantee: stop any currently playing preview
    audio.pause();
    audio.currentTime = 0;
    audio.src = previewUrl;

    setActiveTrackId(trackId);
    setProgress(0);
    setCurrentTime(0);

    audio
      .play()
      .then(() => {
        setIsPlaying(true);
      })
      .catch((err) => {
        setError(`Playback error: ${err.message}`);
        setIsPlaying(false);
        setActiveTrackId(null);
      });
  }, [activeTrackId, isPlaying]);

  const pausePreview = useCallback(() => {
    const audio = audioRef.current;
    if (audio) {
      audio.pause();
      setIsPlaying(false);
    }
  }, []);

  const stopPreview = useCallback(() => {
    const audio = audioRef.current;
    if (audio) {
      audio.pause();
      audio.currentTime = 0;
      setIsPlaying(false);
      setActiveTrackId(null);
      setProgress(0);
      setCurrentTime(0);
    }
  }, []);

  return (
    <AudioContext.Provider
      value={{
        activeTrackId,
        isPlaying,
        progress,
        duration,
        currentTime,
        error,
        playPreview,
        pausePreview,
        stopPreview,
      }}
    >
      {children}
    </AudioContext.Provider>
  );
};

export function useAudioPreview(): AudioContextType {
  const context = useContext(AudioContext);
  if (!context) {
    throw new Error("useAudioPreview must be used within an AudioProvider");
  }
  return context;
}
