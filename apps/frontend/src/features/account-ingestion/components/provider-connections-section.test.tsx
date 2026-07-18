import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/adapters", () => ({
  completePlaidConnection: vi.fn(),
  listProviderConnections: vi.fn(),
  startProviderConnection: vi.fn(),
}));

import { loadPlaidLink, validatedHttpsRedirect } from "./provider-connections-section";

describe("provider connection browser safeguards", () => {
  afterEach(() => {
    document.querySelectorAll("script[src*='cdn.plaid.com']").forEach((script) => script.remove());
    delete window.Plaid;
  });

  it("accepts only credential-free HTTPS redirects", () => {
    expect(validatedHttpsRedirect("https://connect.snaptrade.com/start?id=short-lived")).toBe(
      "https://connect.snaptrade.com/start?id=short-lived",
    );
    expect(() => validatedHttpsRedirect("http://connect.snaptrade.com/start")).toThrow(
      "invalid redirect",
    );
    expect(() => validatedHttpsRedirect("https://user:secret@example.com/start")).toThrow(
      "invalid redirect",
    );
  });

  it("loads Plaid Link only from the official stable HTTPS script", async () => {
    const pending = loadPlaidLink();
    const script = document.querySelector<HTMLScriptElement>("script[src*='cdn.plaid.com']");
    expect(script?.src).toBe("https://cdn.plaid.com/link/v2/stable/link-initialize.js");
    expect(script?.async).toBe(true);

    window.Plaid = { create: vi.fn() } as unknown as NonNullable<Window["Plaid"]>;
    script?.dispatchEvent(new Event("load"));
    await expect(pending).resolves.toBe(window.Plaid);
  });
});
