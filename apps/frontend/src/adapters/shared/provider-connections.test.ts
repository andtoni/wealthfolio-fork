import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("#platform", () => ({
  invoke: mocks.invoke,
}));

import { listProviderConnections, startProviderConnection } from "./provider-connections";

describe("provider-connections shared adapter", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("returns typed provider connection statuses", async () => {
    const fakeResponse = [
      { provider: "investec", status: "not_configured", message: "Not configured yet." },
      { provider: "mercury", status: "not_configured", message: "Not configured yet." },
      { provider: "plaid", status: "not_configured", message: "Not configured yet." },
      { provider: "snaptrade", status: "not_configured", message: "Not configured yet." },
    ];
    mocks.invoke.mockResolvedValue(fakeResponse);

    const result = await listProviderConnections();

    expect(result).toHaveLength(4);
    const providers = result.map((p) => p.provider).sort();
    expect(providers).toEqual(["investec", "mercury", "plaid", "snaptrade"]);

    for (const p of result) {
      expect(p.status).toBe("not_configured");
      expect(p.message).toBeTruthy();
      // No secret/token/apiKey fields — verify at runtime with Object.keys
      const keys = Object.keys(p);
      expect(keys).not.toContain("secret");
      expect(keys).not.toContain("token");
      expect(keys).not.toContain("apiKey");
      expect(keys).not.toContain("accessToken");
    }

    expect(mocks.invoke).toHaveBeenCalledWith("list_provider_connections");
  });

  it("propagates backend errors", async () => {
    mocks.invoke.mockRejectedValue(new Error("Network error"));

    await expect(listProviderConnections()).rejects.toThrow("Network error");
  });

  it("startProviderConnection calls invoke with provider", async () => {
    mocks.invoke.mockResolvedValue(undefined);

    await startProviderConnection("plaid");

    expect(mocks.invoke).toHaveBeenCalledWith("start_provider_connection", { provider: "plaid" });
  });

  it("startProviderConnection propagates errors", async () => {
    mocks.invoke.mockRejectedValue(new Error("Not implemented"));

    await expect(startProviderConnection("investec")).rejects.toThrow("Not implemented");
  });

  it("startProviderConnection accepts all provider kinds", async () => {
    mocks.invoke.mockResolvedValue(undefined);

    for (const provider of ["investec", "mercury", "plaid", "snaptrade"] as const) {
      await startProviderConnection(provider);
      expect(mocks.invoke).toHaveBeenCalledWith("start_provider_connection", { provider });
    }
  });
});
