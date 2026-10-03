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

  it("reconnects the exact Plaid connection that needs re-authentication through update mode", async () => {
    vi.mocked(listProviderConnections).mockResolvedValue([
      {
        provider: "plaid",
        status: "configured",
        message: "Provider is configured.",
        connections: [
          { connectionId: "item-1", label: "Checking •••• 1234", needsReauth: false },
          {
            connectionId: "item-2",
            label: "Venture Card •••• 5678",
            needsReauth: true,
            reason: "ACCOUNT_NOT_RETURNED",
          },
          {
            connectionId: "item-3",
            label: "Savings •••• 9012",
            needsReauth: true,
            reason: "SOME_FUTURE_CODE",
          },
          { connectionId: "item-4", label: "Brokerage •••• 3456", needsReauth: true },
        ],
      },
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
    vi.mocked(startPlaidUpdate).mockResolvedValue({
      provider: "plaid",
      status: "link_ready",
      message: "Plaid Link is ready to add accounts.",
      linkToken: "update-link-token",
      continuationToken: "update-continuation-token",
    });
    vi.mocked(completePlaidUpdate).mockResolvedValue({
      provider: "plaid",
      status: "pending",
      message: "Choose accounts to add.",
    });
    const create = vi.fn((options: Parameters<NonNullable<Window["Plaid"]>["create"]>[0]) => ({
      open: () => options.onSuccess("ignored-public-token"),
      destroy: vi.fn(),
    }));
    window.Plaid = { create };
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={queryClient}>
        <ProviderConnectionsSection />
      </QueryClientProvider>,
    );

    // The card says so plainly: only the account row still reads "Connected".
    expect(await screen.findByText("Reconnect needed")).toBeTruthy();
    expect(screen.getAllByText("Connected")).toHaveLength(1);
    expect(screen.getByText("Venture Card •••• 5678")).toBeTruthy();
    expect(screen.queryByText("Checking •••• 1234")).toBeNull();
    expect(screen.getByRole("button", { name: "Add accounts" })).toBeTruthy();
    // Each connection explains itself from its fixed reason code; unknown or absent codes read generically.
    expect(
      screen.getByText(
        "The bank stopped sharing an account you added. Reconnect and select it again.",
      ),
    ).toBeTruthy();
    expect(screen.getAllByText("This bank login needs to be reconnected.")).toHaveLength(2);
    expect(screen.queryByText(/SOME_FUTURE_CODE|ACCOUNT_NOT_RETURNED/)).toBeNull();

    // One Reconnect button per connection that needs it: the healthy connection offers none.
    const reconnect = screen.getAllByRole("button", { name: "Reconnect" });
    expect(reconnect).toHaveLength(3);
    fireEvent.click(reconnect[0]);

    await waitFor(() => expect(startPlaidUpdate).toHaveBeenCalledWith("item-2"));
    await waitFor(() =>
      expect(completePlaidUpdate).toHaveBeenCalledWith("update-continuation-token"),
    );
    expect(create).toHaveBeenCalledWith(expect.objectContaining({ token: "update-link-token" }));
    expect(startProviderConnection).not.toHaveBeenCalled();
    expect(completePlaidConnection).not.toHaveBeenCalled();
    await waitFor(() => expect(listProviderConnections).toHaveBeenCalledTimes(2));
  });

  it("shows a connected Plaid card without a reconnect prompt when no connection needs it", async () => {
    vi.mocked(listProviderConnections).mockResolvedValue([
      {
        provider: "plaid",
        status: "configured",
        message: "Provider is configured.",
        connections: [{ connectionId: "item-1", label: "Checking •••• 1234", needsReauth: false }],
      },
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
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={queryClient}>
        <ProviderConnectionsSection />
      </QueryClientProvider>,
    );

    await screen.findByRole("button", { name: "Add accounts" });
    expect(screen.getAllByText("Connected")).toHaveLength(2);
    expect(screen.queryByText("Reconnect needed")).toBeNull();
    expect(screen.queryByRole("button", { name: "Reconnect" })).toBeNull();
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
