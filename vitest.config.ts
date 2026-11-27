import { mergeConfig, defineConfig } from "vitest/config";
import viteConfig from "./vite.config";

export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      exclude: [
        "**/node_modules/**",
        "**/dist/**",
        "**/.kilo/**",
        "**/.planning/**",
        "**/src-tauri/**",
      ],
    },
  }),
);
