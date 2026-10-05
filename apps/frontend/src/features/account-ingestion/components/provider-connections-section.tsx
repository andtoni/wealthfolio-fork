import {
  admitProviderAccount,
  completePlaidConnection,
  completePlaidUpdate,
  getAccounts,
  listDiscoveredProviderAccounts,
  listProviderConnections,
  startPlaidUpdate,
  startProviderConnection,
  type ProviderConnectionStatus,
} from "@/adapters";
import { SECTION_HEADING } from "@/components/dashboard-card";
import type { Account } from "@/lib/types";
import { Card, CardContent, Icons } from "@wealthfolio/ui";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";

const PLAID_SCRIPT_URL = "https://cdn.plaid.com/link/v2/stable/link-initialize.js";

interface PlaidHandler {
  open(): void;
  destroy(): void;
}

interface PlaidApi {
  create(options: {
    token: string;
    onSuccess(publicToken: string): void;
    onExit(error: { display_message?: string | null } | null): void;
  }): PlaidHandler;
}

declare global {
  interface Window {
    Plaid?: PlaidApi;
  }
}

let plaidScriptPromise: Promise<PlaidApi> | undefined;

export function loadPlaidLink(): Promise<PlaidApi> {
  if (window.Plaid) return Promise.resolve(window.Plaid);
  if (plaidScriptPromise) return plaidScriptPromise;

  const promise = new Promise<PlaidApi>((resolve, reject) => {
    const existing = document.querySelector<HTMLScriptElement>(`script[src="${PLAID_SCRIPT_URL}"]`);
    const script = existing ?? document.createElement("script");
    const onLoad = () => {
      if (window.Plaid) resolve(window.Plaid);
      else reject(new Error("Plaid Link did not initialize."));
    };
    const onError = () => reject(new Error("Unable to load Plaid Link."));
    script.addEventListener("load", onLoad, { once: true });
    script.addEventListener("error", onError, { once: true });
    if (!existing) {
      script.src = PLAID_SCRIPT_URL;
      script.async = true;
      document.head.appendChild(script);
    }
  }).catch((error: unknown) => {
    plaidScriptPromise = undefined;
    throw error;
  });
  plaidScriptPromise = promise;
  return promise;
}

export function validatedHttpsRedirect(value: string): string {
  const redirect = new URL(value);
  if (
    redirect.protocol !== "https:" ||
    redirect.hostname !== "app.snaptrade.com" ||
    redirect.pathname !== "/snapTrade/redeemToken" ||
    redirect.username ||
    redirect.password
  ) {
    throw new Error("Provider returned an invalid redirect.");
  }
  return redirect.href;
}

export async function openPlaid(
  linkToken: string,
  continuationToken: string,
  update = false,
): Promise<void> {
  if (!linkToken || !continuationToken)
    throw new Error("Plaid did not return a complete link session.");
  const plaid = await loadPlaidLink();
  await new Promise<void>((resolve, reject) => {
    let completionStarted = false;
    const handler: PlaidHandler = plaid.create({
      token: linkToken,
      onSuccess: (publicToken) => {
        completionStarted = true;
        const completion = update
          ? completePlaidUpdate(continuationToken)
          : completePlaidConnection(publicToken, continuationToken);
        void completion
          .then(() => resolve())
          .catch(reject)
          .finally(() => handler.destroy());
      },
      onExit: (error) => {
        if (completionStarted) return;
        handler.destroy();
        reject(
          new Error(
            error?.display_message ||
              "Plaid Link closed before the connection completed. Reopen it and finish the final confirmation.",
          ),
        );
      },
    });
    handler.open();
  });
}

export function openPlaidUpdate(linkToken: string, continuationToken: string): Promise<void> {
  return openPlaid(linkToken, continuationToken, true);
}

const RECONNECT_REASONS: Record<string, string> = {
  ITEM_LOGIN_REQUIRED: "The bank needs you to log in again.",
  ACCESS_NOT_GRANTED:
    "The bank is not sharing the access this connection needs. Reconnect and allow it.",
  NO_ACCOUNTS:
    "The bank is no longer sharing any accounts for this login. Reconnect and select them.",
  ACCOUNT_NOT_RETURNED:
    "The bank stopped sharing an account you added. Reconnect and select it again.",
  ITEM_LOCKED: "The bank has locked this login. Unlock it with the bank, then reconnect.",
  PASSWORD_RESET_REQUIRED:
    "The bank requires a password reset. Reset it with the bank, then reconnect.",
  USER_SETUP_REQUIRED: "The bank needs you to finish a step on its website, then reconnect.",
  ITEM_NOT_FOUND: "This bank login no longer exists. Use Add accounts to connect it again.",
};

function reconnectReason(reason?: string): string {
  return (
    (reason && Object.hasOwn(RECONNECT_REASONS, reason) && RECONNECT_REASONS[reason]) ||
    "This bank login needs to be reconnected."
  );
}

const PROVIDER_NAMES: Record<string, string> = {
  investec: "Investec",
  mercury: "Mercury",
  plaid: "Plaid",
  snaptrade: "SnapTrade",
};

const PROVIDER_ICONS: Record<string, React.ReactNode> = {
  investec: <Icons.Building className="h-5 w-5" />,
  mercury: <Icons.Banknote className="h-5 w-5" />,
  plaid: <Icons.Link className="h-5 w-5" />,
  snaptrade: <Icons.ArrowLeftRight className="h-5 w-5" />,
};

function ProviderCard({
  status,
  connectedAccounts,
}: {
  status: ProviderConnectionStatus;
  connectedAccounts: Account[];
}) {
  const name = PROVIDER_NAMES[status.provider] ?? status.provider;
  const icon = PROVIDER_ICONS[status.provider] ?? <Icons.Link className="h-5 w-5" />;
  const queryClient = useQueryClient();
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const providerReportsConnected = status.status === "connected" || status.status === "completed";
  const visibleConnectedAccounts =
    status.provider === "snaptrade" && !providerReportsConnected ? [] : connectedAccounts;
  const isConnected = visibleConnectedAccounts.length > 0;
  const plaidConnectionConfigured =
    status.provider === "plaid" && (status.status === "configured" || providerReportsConnected);
  const supportsConnect = status.provider === "plaid" || !isConnected;
  const canConnect =
    status.status === "available" ||
    status.status === "configured" ||
    (status.provider === "plaid" && providerReportsConnected);
  const canDiscover = ["available", "configured", "connected", "completed"].includes(status.status);
  const { data: discovered = [] } = useQuery({
    queryKey: ["provider-accounts", status.provider],
    queryFn: () => listDiscoveredProviderAccounts(status.provider),
    staleTime: 15_000,
    enabled: canDiscover,
  });

  const startMutation = useMutation({
    mutationFn: async () => {
      // Add accounts must start a new Plaid Item so Link opens institution search.
      // Plaid update mode is Item-bound and therefore cannot connect another bank.
      const result = await startProviderConnection(status.provider);
      if (status.provider === "snaptrade" && result.status === "link_ready") {
        if (!result.redirectUrl) throw new Error("SnapTrade did not return a redirect.");
        window.location.assign(validatedHttpsRedirect(result.redirectUrl));
        return;
      }
      if (status.provider === "plaid") {
        await openPlaid(result.linkToken ?? "", result.continuationToken ?? "");
      }
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["provider-connections"] });
      void queryClient.invalidateQueries({ queryKey: ["provider-accounts", status.provider] });
    },
    onError: (error: Error) => {
      setErrorMsg(error.message || "Provider connection could not be started.");
    },
  });

  const reauthConnections = (status.connections ?? []).filter(
    (connection) => connection.needsReauth,
  );
  const reconnectMutation = useMutation({
    mutationFn: async (connectionId: string) => {
      // Update mode re-authenticates the same Plaid Item. Starting a new
      // connection for a bank login that only needs refreshing duplicates it.
      const result = await startPlaidUpdate(connectionId);
      await openPlaidUpdate(result.linkToken ?? "", result.continuationToken ?? "");
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["provider-connections"] });
      void queryClient.invalidateQueries({ queryKey: ["provider-accounts", status.provider] });
    },
    onError: (error: Error) => {
      setErrorMsg(error.message || "Bank connection could not be reconnected.");
    },
  });

  const admitMutation = useMutation({
    mutationFn: (account: (typeof discovered)[number]) =>
      admitProviderAccount(status.provider, account),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["provider-accounts", status.provider] });
      void queryClient.invalidateQueries({ queryKey: ["provider-connections"] });
      void queryClient.invalidateQueries({ queryKey: ["accounts"] });
    },
    onError: (error: Error) => setErrorMsg(error.message || "Account could not be added."),
  });

  const connectionIsHealthy = !["auth_required", "unavailable", "not_configured"].includes(
    status.status,
  );
  const stateLabel =
    reauthConnections.length > 0
      ? "Reconnect needed"
      : isConnected && connectionIsHealthy
        ? "Connected"
        : status.status === "configured"
          ? "Configured"
          : status.status.replaceAll("_", " ");
  const connectLabel = plaidConnectionConfigured && isConnected ? "Add accounts" : "Connect";

  return (
    <Card>
      <CardContent className="flex flex-col gap-2 p-0">
        <div className="flex items-center gap-3 py-2.5">
          <div className="bg-foreground/[0.07] flex h-10 w-10 shrink-0 items-center justify-center rounded-full">
            {icon}
          </div>
          <div className="min-w-0 flex-1">
            <p className="truncate text-[17px] leading-snug">{name}</p>
            <p className="text-muted-foreground text-[15px] leading-snug">
              <span className="inline-block first-letter:uppercase">{stateLabel}</span>
            </p>
            <p className="text-muted-foreground hidden text-[13px] leading-snug sm:block">
              {status.message}
            </p>
          </div>
          {supportsConnect && (
            <button
              type="button"
              disabled={!canConnect || startMutation.isPending}
              onClick={() => {
                setErrorMsg(null);
                startMutation.mutate();
              }}
              className="bg-foreground text-background inline-flex h-9 shrink-0 items-center rounded-full px-4 text-sm transition-opacity disabled:opacity-40"
            >
              {startMutation.isPending ? "Connecting..." : connectLabel}
            </button>
          )}
        </div>
        {reauthConnections.length > 0 && (
          <div className="space-y-1 pl-[52px]">
            <p className="text-muted-foreground text-[13px]">Bank logins to reconnect</p>
            {reauthConnections.map((connection) => (
              <div
                key={connection.connectionId}
                className="border-warning/30 bg-warning/10 flex items-center gap-2 rounded-md border px-3 py-2"
              >
                <div className="min-w-0 flex-1">
                  <p className="truncate text-[15px] leading-snug">{connection.label}</p>
                  <p className="text-muted-foreground text-[13px] leading-snug">
                    {reconnectReason(connection.reason)}
                  </p>
                </div>
                <button
                  type="button"
                  disabled={reconnectMutation.isPending}
                  onClick={() => {
                    setErrorMsg(null);
                    reconnectMutation.mutate(connection.connectionId);
                  }}
                  className="bg-foreground text-background inline-flex h-8 items-center rounded-full px-3.5 text-sm disabled:opacity-40"
                >
                  {reconnectMutation.isPending &&
                  reconnectMutation.variables === connection.connectionId
                    ? "Reconnecting..."
                    : "Reconnect"}
                </button>
              </div>
            ))}
          </div>
        )}
        {visibleConnectedAccounts.length > 0 && (
          <div className="space-y-1 pl-[52px]">
            <p className="text-muted-foreground text-[13px]">Connected accounts</p>
            {visibleConnectedAccounts.map((account) => (
              <div key={account.id} className="flex items-center gap-2 py-1.5">
                <div className="min-w-0 flex-1">
                  <p className="truncate text-[15px] leading-snug">{account.name}</p>
                  <p className="text-muted-foreground text-[13px] leading-snug">
                    {account.accountType.replaceAll("_", " ").toLowerCase()}, {account.currency}
                  </p>
                </div>
                <span className="text-success text-[13px]">Connected</span>
              </div>
            ))}
          </div>
        )}
        {discovered.length > 0 && (
          <div className="space-y-1 pl-[52px]">
            <p className="text-muted-foreground text-[13px]">Choose accounts to add</p>
            {discovered.map((account) => (
              <div
                key={`${account.connectionId}:${account.providerAccountId}`}
                className="flex items-center gap-2 py-1.5"
              >
                <div className="min-w-0 flex-1">
                  <p className="truncate text-[15px] leading-snug">{account.name}</p>
                  <p className="text-muted-foreground text-[13px] leading-snug">
                    {account.type} · {account.currency}
                    {account.mask ? ` · •••• ${account.mask}` : ""}
                    {status.provider === "snaptrade"
                      ? account.assets.length === 0
                        ? " · Cash only"
                        : ` · ${account.assets.length} asset${account.assets.length === 1 ? "" : "s"}`
                      : ""}
                  </p>
                </div>
                <button
                  type="button"
                  disabled={admitMutation.isPending}
                  onClick={() => {
                    setErrorMsg(null);
                    admitMutation.mutate(account);
                  }}
                  className="bg-foreground text-background inline-flex h-8 items-center rounded-full px-3.5 text-sm disabled:opacity-40"
                >
                  {admitMutation.isPending ? "Adding..." : "Add"}
                </button>
              </div>
            ))}
          </div>
        )}
        {errorMsg && (
          <div className="bg-destructive/10 text-destructive rounded-md px-3 py-2 text-xs">
            {errorMsg}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

const PROVIDER_FALLBACKS: ProviderConnectionStatus[] = [
  { provider: "plaid", status: "unavailable", message: "Bank connection is not enabled yet." },
  {
    provider: "snaptrade",
    status: "unavailable",
    message: "Brokerage connection requires qualified live access.",
  },
  { provider: "investec", status: "unavailable", message: "Bank connection is not enabled yet." },
  { provider: "mercury", status: "unavailable", message: "Bank connection is not enabled yet." },
];

export function ProviderConnectionsSection() {
  const { t } = useTranslation();
  const { data, isError, isPending } = useQuery({
    queryKey: ["provider-connections"],
    queryFn: listProviderConnections,
    staleTime: 30_000,
    retry: 1,
  });
  const { data: accounts = [] } = useQuery<Account[], Error>({
    queryKey: ["accounts"],
    queryFn: () => getAccounts(),
  });
  const statuses = new Map((data ?? []).map((status) => [status.provider, status]));
  const providers = PROVIDER_FALLBACKS.map(
    (fallback) => statuses.get(fallback.provider) ?? fallback,
  );

  if (isPending) {
    return (
      <div className="space-y-2" aria-live="polite">
        <h3 className={SECTION_HEADING}>
          {t("connect:directProviders", "Direct Provider Connections")}
        </h3>
        <div className="bg-muted h-24 animate-pulse rounded-lg" />
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <h3 className={SECTION_HEADING}>
        {t("connect:directProviders", "Direct Provider Connections")}
      </h3>
      <p className="text-muted-foreground -mt-2 text-[15px]">
        {t(
          "connect:directProvidersDescription",
          "Connect your bank or brokerage directly for automatic syncing. Each provider requires separate setup.",
        )}
      </p>
      {isError && (
        <p className="border-warning/30 bg-warning/10 rounded-md border px-3 py-2 text-xs">
          Automatic connections are currently locked while provider activation is completed. Manual
          accounts remain available below.
        </p>
      )}
      <div className="grid grid-cols-1 gap-x-10 md:grid-cols-2">
        {providers.map((provider) => (
          <ProviderCard
            key={provider.provider}
            status={provider}
            connectedAccounts={accounts.filter(
              (account) =>
                !account.isArchived && account.provider?.toLowerCase() === provider.provider,
            )}
          />
        ))}
      </div>
    </div>
  );
}
