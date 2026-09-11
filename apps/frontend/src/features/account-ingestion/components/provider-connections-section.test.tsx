import { afterEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

vi.mock("@/adapters", () => ({
  admitProviderAccount: vi.fn(),
  completePlaidConnection: vi.fn(),
  completePlaidUpdate: vi.fn(),
  getAccounts: vi.fn().mockResolvedValue([]),
  listDiscoveredProviderAccounts: vi.fn().mockResolvedValue([]),
  listProviderConnections: vi.fn(),
  startPlaidUpdate: vi.fn(),
  startProviderConnection: vi.fn(),
}));

import {
  completePlaidConnection,
  completePlaidUpdate,
  getAccounts,
  listProviderConnections,
  startPlaidUpdate,
  startProviderConnection,
} from "@/adapters";
import {
  loadPlaidLink,
  openPlaid,
  openPlaidUpdate,
  ProviderConnectionsSection,
  validatedHttpsRedirect,
} from "./provider-connections-section";

describe("provider connection browser safeguards", () => {
  it("starts a new Plaid connection when Add accounts is clicked", async () => {
    vi.mocked(listProviderConnections).mockResolvedValue([
      { provider: "plaid", status: "completed", message: "Connection completed." },
    ]);
    vi.mocked(getAccounts).mockResolvedValue([
      {
        id: "plaid-account",
        name: "Connected checking",
        provider: "plaid",
        isArchived: false,
        accountType: "CASH",
        currency: "USD",
      } as never,
    ]);
    vi.mocked(startProviderConnection).mockResolvedValue({
      provider: "plaid",
      status: "link_ready",
      message: "Plaid Link is ready.",
      linkToken: "new-link-token",
      continuationToken: "new-continuation-token",
    });
    vi.mocked(completePlaidConnection).mockResolvedValue({
      provider: "plaid",
      status: "pending",
      message: "Choose accounts to add.",
    });
    window.Plaid = {
      create: vi.fn((options) => ({
        open: () => options.onSuccess("public-token"),
        destroy: vi.fn(),
      })),
    };
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={queryClient}>
        <ProviderConnectionsSection />
      </QueryClientProvider>,
    );

    const button = await screen.findByRole("button", { name: "Add accounts" });
    fireEvent.click(button);

    await waitFor(() => expect(startProviderConnection).toHaveBeenCalledWith("plaid"));
    expect(startPlaidUpdate).not.toHaveBeenCalled();
    await waitFor(() =>
      expect(completePlaidConnection).toHaveBeenCalledWith(
        "public-token",
        "new-continuation-token",
      ),
    );
  });

  afterEach(() => {
    document.querySelectorAll("script[src*='cdn.plaid.com']").forEach((script) => script.remove());
    delete window.Plaid;
    vi.clearAllMocks();
  });

  it("accepts only credential-free HTTPS redirects", () => {
    const official = "https://app.snaptrade.com/snapTrade/redeemToken?token=short-lived";
    expect(validatedHttpsRedirect(official)).toBe(official);
    expect(() => validatedHttpsRedirect("http://app.snaptrade.com/snapTrade/redeemToken")).toThrow(
      "invalid redirect",
    );
    expect(() => validatedHttpsRedirect("https://example.com/snapTrade/redeemToken")).toThrow(
      "invalid redirect",
    );
    expect(() =>
      validatedHttpsRedirect("https://user:secret@app.snaptrade.com/snapTrade/redeemToken"),
    ).toThrow("invalid redirect");
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

  it("does not report success when Link exits before its completion callback", async () => {
    let callbacks: Parameters<NonNullable<Window["Plaid"]>["create"]>[0] | undefined;
    window.Plaid = {
      create: vi.fn((options) => {
        callbacks = options;
        return { open: vi.fn(), destroy: vi.fn() };
      }),
    };

    const pending = openPlaid("link-token", "continuation-token");
    await vi.waitFor(() => expect(callbacks).toBeDefined());
    callbacks?.onExit(null);

    await expect(pending).rejects.toThrow("closed before the connection completed");
    expect(completePlaidConnection).not.toHaveBeenCalled();
  });

  it("completes the server exchange only after Plaid onSuccess", async () => {
    let callbacks: Parameters<NonNullable<Window["Plaid"]>["create"]>[0] | undefined;
    vi.mocked(completePlaidConnection).mockResolvedValue({
      provider: "plaid",
      status: "pending",
      message: "Choose accounts to add.",
    });
    window.Plaid = {
      create: vi.fn((options) => {
        callbacks = options;
        return { open: vi.fn(), destroy: vi.fn() };
      }),
    };

    const pending = openPlaid("link-token", "continuation-token");
    await vi.waitFor(() => expect(callbacks).toBeDefined());
    callbacks?.onSuccess("public-token");

    await expect(pending).resolves.toBeUndefined();
    expect(completePlaidConnection).toHaveBeenCalledWith("public-token", "continuation-token");
  });

  it("uses the update completion without exchanging a public token", async () => {
    let callbacks: Parameters<NonNullable<Window["Plaid"]>["create"]>[0] | undefined;
    vi.mocked(completePlaidUpdate).mockResolvedValue({
      provider: "plaid",
      status: "pending",
      message: "Choose accounts to add.",
    });
    window.Plaid = {
      create: vi.fn((options) => {
        callbacks = options;
        return { open: vi.fn(), destroy: vi.fn() };
      }),
    };

    const pending = openPlaidUpdate("update-link-token", "continuation-token");
    await vi.waitFor(() => expect(callbacks).toBeDefined());
    callbacks?.onSuccess("ignored-public-token");

    await expect(pending).resolves.toBeUndefined();
    expect(completePlaidUpdate).toHaveBeenCalledWith("continuation-token");
    expect(completePlaidConnection).not.toHaveBeenCalled();
  });
});
