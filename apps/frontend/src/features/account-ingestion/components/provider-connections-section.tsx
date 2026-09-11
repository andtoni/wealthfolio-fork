import {
  admitProviderAccount,
  completePlaidConnection,
  completePlaidUpdate,
  getAccounts,
  listDiscoveredProviderAccounts,
  listProviderConnections,
  startProviderConnection,
  type ProviderConnectionStatus,
} from "@/adapters";
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
    isConnected && connectionIsHealthy
      ? "Connected"
      : status.status === "configured"
        ? "Configured"
        : status.status.replaceAll("_", " ");
  const connectLabel = plaidConnectionConfigured && isConnected ? "Add accounts" : "Connect";

  return (
    <Card className="border">
      <CardContent className="flex flex-col gap-3 p-4">
        <div className="flex items-center gap-3">
          <div className="bg-muted flex h-10 w-10 shrink-0 items-center justify-center rounded-lg">
            {icon}
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-sm font-medium leading-tight">{name}</p>
            <p className="text-muted-foreground mt-0.5 truncate text-xs leading-tight">
              {status.message}
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <span className="bg-muted text-muted-foreground inline-flex items-center rounded-full px-2.5 py-0.5 text-[10px] font-medium uppercase tracking-wider">
              {stateLabel}
            </span>
            {supportsConnect && (
              <button
                type="button"
                disabled={!canConnect || startMutation.isPending}
                onClick={() => {
                  setErrorMsg(null);
                  startMutation.mutate();
                }}
                className="bg-primary text-primary-foreground hover:bg-primary/90 inline-flex h-8 items-center rounded-md px-3 text-xs font-medium transition-colors disabled:opacity-50"
              >
                {startMutation.isPending ? "Connecting..." : connectLabel}
              </button>
            )}
          </div>
        </div>
        {visibleConnectedAccounts.length > 0 && (
          <div className="space-y-2 border-t pt-3">
            <p className="text-muted-foreground text-xs font-medium">Connected accounts</p>
            {visibleConnectedAccounts.map((account) => (
              <div key={account.id} className="flex items-center gap-2 rounded-md border px-3 py-2">
                <div className="min-w-0 flex-1">
                  <p className="truncate text-xs font-medium">{account.name}</p>
                  <p className="text-muted-foreground text-[11px]">
                    {account.accountType.replaceAll("_", " ")} · {account.currency}
                  </p>
                </div>
                <span className="bg-success/10 text-success rounded-full px-2 py-0.5 text-[10px] font-medium">
                  Connected
                </span>
              </div>
            ))}
          </div>
        )}
        {discovered.length > 0 && (
          <div className="space-y-2 border-t pt-3">
            <p className="text-muted-foreground text-xs font-medium">Choose accounts to add</p>
            {discovered.map((account) => (
              <div
                key={`${account.connectionId}:${account.providerAccountId}`}
                className="flex items-center gap-2 rounded-md border px-3 py-2"
              >
                <div className="min-w-0 flex-1">
                  <p className="truncate text-xs font-medium">{account.name}</p>
                  <p className="text-muted-foreground text-[11px]">
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
                  className="bg-primary text-primary-foreground hover:bg-primary/90 inline-flex h-8 items-center rounded-md px-3 text-xs font-medium disabled:opacity-50"
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
        <h3 className="text-muted-foreground text-xs font-semibold uppercase tracking-wider">
          {t("connect:directProviders", "Direct Provider Connections")}
        </h3>
        <div className="bg-muted h-24 animate-pulse rounded-lg" />
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <h3 className="text-muted-foreground text-xs font-semibold uppercase tracking-wider">
        {t("connect:directProviders", "Direct Provider Connections")}
      </h3>
      <p className="text-muted-foreground -mt-2 text-xs">
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
      <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
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
