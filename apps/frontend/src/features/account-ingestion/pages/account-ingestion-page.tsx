import appLogo from "@/assets/logo-rounded.png";
import { Page, PageContent, PageHeader } from "@/components/page";
import { ProviderConnectionsSection } from "@/features/account-ingestion/components/provider-connections-section";
import {
  Avatar,
  AvatarFallback,
  AvatarImage,
  Button,
  Card,
  CardContent,
  Icons,
} from "@wealthfolio/ui";
import { useAuth } from "@/context/auth-context";
import { useTranslation } from "react-i18next";
import { useEffect, useState } from "react";
import { Link } from "react-router-dom";

/** Shared content rendered in both /connect and /settings/connect */
export function IngestionHubContent() {
  const { t } = useTranslation();
  const { isAuthenticated, requiresAuth } = useAuth();
  const [subject, setSubject] = useState<string | null>(null);

  useEffect(() => {
    if (requiresAuth && isAuthenticated) {
      fetch("/api/v1/auth/me", { credentials: "same-origin" })
        .then((r) => (r.ok ? r.json() : null))
        .then((data) => {
          if (data?.subject) setSubject(data.subject);
        })
        .catch(() => {});
    }
  }, [requiresAuth, isAuthenticated]);

  return (
    <div className="mx-auto max-w-5xl space-y-6 py-6 lg:py-8">
      {/* Logo + Welcome */}
      <div className="flex flex-col items-center text-center">
        <Avatar className="mb-4 h-20 w-20">
          <AvatarImage src={appLogo} alt="Wealthfolio" />
          <AvatarFallback>WF</AvatarFallback>
        </Avatar>
        <h2 className="text-foreground text-xl font-semibold">
          {t("connect:connect.title", "Connect")}
        </h2>
        <p className="text-muted-foreground mt-2 max-w-md text-sm">
          {t(
            "connect:connect.description",
            "Link your brokerage accounts or import data manually. Everything stays on your device.",
          )}
        </p>
      </div>

      {/* Provider connections are the primary account-ingestion path. */}
      <ProviderConnectionsSection />

      {/* Manual and file-based fallbacks */}
      <Card>
        <CardContent className="flex flex-col items-center gap-4 p-6 text-center sm:p-8">
          <div className="bg-muted flex h-16 w-16 items-center justify-center rounded-full">
            <Icons.Link className="text-muted-foreground h-7 w-7" />
          </div>
          <div>
            <h3 className="text-lg font-medium">
              {t("connect:connect.noAccounts", "Other ways to add accounts")}
            </h3>
            <p className="text-muted-foreground mt-1 text-sm">
              {t(
                "connect:connect.noAccountsDescription",
                "Create a manual account or import a file when a direct provider is not available.",
              )}
            </p>
          </div>
          <div className="flex flex-wrap justify-center gap-3">
            <Button asChild size="lg">
              <Link to="/settings/accounts" state={{ newAccount: true }}>
                <Icons.Plus className="mr-2 h-4 w-4" />
                {t("connect:connect.addAccount", "Add Manual Account")}
              </Link>
            </Button>
            <Button asChild variant="outline" size="lg">
              <Link to="/import">
                <Icons.Upload className="mr-2 h-4 w-4" />
                {t("connect:connect.csvImport", "Import CSV")}
              </Link>
            </Button>
          </div>
        </CardContent>
      </Card>

      {/* Secure local status */}
      <Card className="border-primary/20 bg-primary/5">
        <CardContent className="flex items-center gap-4 p-6">
          <div className="bg-primary/10 flex h-12 w-12 shrink-0 items-center justify-center rounded-full">
            <Icons.Shield className="text-primary h-6 w-6" />
          </div>
          <div>
            <h4 className="text-sm font-medium">
              {t("connect:connect.secureTitle", "Your Data Stays Local")}
            </h4>
            <p className="text-muted-foreground text-xs">
              {t(
                "connect:connect.secureDescription",
                "All your financial data is stored securely on your device. Nothing is sent to external servers unless you explicitly export or sync.",
              )}
            </p>
          </div>
        </CardContent>
      </Card>

      {/* Authenticated subject badge */}
      {subject && (
        <div className="border-muted-foreground/20 text-muted-foreground mx-auto flex items-center gap-2 border-t pt-4 text-center text-[11px]">
          <Icons.Shield className="h-3 w-3" />
          <span>
            Authenticated as:{" "}
            <code className="bg-muted rounded px-1 py-0.5 font-mono text-[10px]">{subject}</code>
          </span>
        </div>
      )}
    </div>
  );
}

export default function AccountIngestionPage() {
  const { t } = useTranslation();

  return (
    <Page>
      <PageHeader heading={t("connect:connect.title", "Connect")} />
      <PageContent>
        <IngestionHubContent />
      </PageContent>
    </Page>
  );
}
