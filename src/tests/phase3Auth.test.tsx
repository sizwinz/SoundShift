// @vitest-environment jsdom
import { act } from "react";
import { createRoot, Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AuthProvider, useAuth } from "../context/AuthContext";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));
vi.mock("../utils/tauri", () => ({ isTauri: () => true }));

let root: Root;
let container: HTMLDivElement;

beforeEach(() => {
  (
    globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }
  ).IS_REACT_ACT_ENVIRONMENT = true;
  mocks.invoke.mockReset();
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

describe("Phase 3 live AuthContext verification", () => {
  it("transitions stale connected session to expired when check_provider_connection rejects with 401 authentication error", async () => {
    mocks.invoke.mockImplementation(async (command: string, args?: { service?: string }) => {
      if (command === "check_auth") {
        return { service: args?.service, status: "connected" };
      }
      if (command === "check_provider_connection") {
        if (args?.service === "spotify") {
          throw new Error("Provider Authentication (HTTP 401); review the recorded operation before retry");
        }
        return null;
      }
      return null;
    });

    function Probe() {
      const { spotifyStatus, ytStatus } = useAuth();
      return (
        <div>
          <span data-testid="sp">{spotifyStatus}</span>
          <span data-testid="yt">{ytStatus}</span>
        </div>
      );
    }

    await act(async () => {
      root.render(
        <AuthProvider>
          <Probe />
        </AuthProvider>,
      );
    });

    await act(async () => {
      await Promise.resolve();
    });

    expect(container.querySelector('[data-testid="sp"]')?.textContent).toBe("expired");
    expect(container.querySelector('[data-testid="yt"]')?.textContent).toBe("connected");
  });
});
