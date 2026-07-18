import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("#platform", () => ({
  invoke: mocks.invoke,
}));

import {
  completePlaidConnection,
  listProviderConnections,
  startProviderConnection,
} from "./provider-connections";

describe("provider-connections shared adapter", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("returns typed provider connection statuses without secret fields", async () => {
    const response = [
      { provider: "investec", status: "configured", message: "Configured." },
      { provider: "mercury", status: "configured", message: "Configured." },
      { provider: "plaid", status: "configured", message: "Ready." },
      { provider: "snaptrade", status: "configured", message: "Ready." },
    ];
    mocks.invoke.mockResolvedValue(response);

    const result = await listProviderConnections();

    expect(result).toEqual(response);
    expect(mocks.invoke).toHaveBeenCalledWith("list_provider_connections");
    for (const provider of result) {
      expect(Object.keys(provider)).not.toContain("accessToken");
      expect(Object.keys(provider)).not.toContain("secret");
    }
  });

  it("returns only the short-lived provider start material", async () => {
    const response = {
      provider: "plaid",
      status: "link_ready",
      message: "Ready.",
      linkToken: "link-short-lived",
    };
    mocks.invoke.mockResolvedValue(response);

    await expect(startProviderConnection("plaid")).resolves.toEqual(response);
    expect(mocks.invoke).toHaveBeenCalledWith("start_provider_connection", { provider: "plaid" });
  });

  it("sends a Plaid public token only to the completion command", async () => {
    const response = { provider: "plaid", status: "completed", message: "Connected." };
    mocks.invoke.mockResolvedValue(response);

    await expect(completePlaidConnection("public-short-lived")).resolves.toEqual(response);
    expect(mocks.invoke).toHaveBeenCalledWith("complete_plaid_connection", {
      publicToken: "public-short-lived",
    });
  });

  it("propagates backend failures", async () => {
    mocks.invoke.mockRejectedValue(new Error("Provider connection service is unavailable."));

    await expect(listProviderConnections()).rejects.toThrow("unavailable");
    await expect(startProviderConnection("snaptrade")).rejects.toThrow("unavailable");
    await expect(completePlaidConnection("public-token")).rejects.toThrow("unavailable");
  });
});
