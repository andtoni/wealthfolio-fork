import {
  listProviderConnections,
  startProviderConnection,
  type ProviderConnectionStatus,
} from "@/adapters";
import { Card, CardContent } from "@wealthfolio/ui/components/ui/card";
import { Icons } from "@wealthfolio/ui/components/ui/icons";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useTranslation } from "react-i18next";

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

function ProviderCard({ status }: { status: ProviderConnectionStatus }) {
  const name = PROVIDER_NAMES[status.provider] ?? status.provider;
  const icon = PROVIDER_ICONS[status.provider] ?? <Icons.Link className="h-5 w-5" />;
  const queryClient = useQueryClient();
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const startMutation = useMutation({
    mutationFn: () => startProviderConnection(status.provider),
    onSuccess: () => {
      setErrorMsg(
        "Provider is not yet available for direct linking. Use CSV import or manually add accounts instead.",
      );
      queryClient.invalidateQueries({ queryKey: ["provider-connections"] });
    },
    onError: (err: Error) => {
      setErrorMsg(
        err.message ||
          "Provider linking is not yet implemented. No URL, token, or credentials were generated.",
      );
    },
  });

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
              {status.status === "not_configured" ? "Off" : status.status}
            </span>
            <button
              type="button"
              disabled={startMutation.isPending}
              onClick={() => {
                setErrorMsg(null);
                startMutation.mutate();
              }}
              className="bg-primary text-primary-foreground hover:bg-primary/90 inline-flex h-8 items-center rounded-md px-3 text-xs font-medium transition-colors disabled:opacity-50"
            >
              {startMutation.isPending ? "Connecting..." : "Connect"}
            </button>
          </div>
        </div>
        {errorMsg && (
          <div className="bg-destructive/10 text-destructive rounded-md px-3 py-2 text-xs">
            {errorMsg}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

export function ProviderConnectionsSection() {
  const { t } = useTranslation();
  const { data: providers = [] } = useQuery({
    queryKey: ["provider-connections"],
    queryFn: listProviderConnections,
    staleTime: 30_000,
  });

  if (providers.length === 0) return null;

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
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        {providers.map((p) => (
          <ProviderCard key={p.provider} status={p} />
        ))}
      </div>
    </div>
  );
}
