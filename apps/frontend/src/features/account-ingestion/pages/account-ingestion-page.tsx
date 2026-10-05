import { LIST_ROW, SECTION_HEADING } from "@/components/dashboard-card";
import { Page, PageContent, PageHeader } from "@/components/page";
import { ProviderConnectionsSection } from "@/features/account-ingestion/components/provider-connections-section";
import { Icons } from "@wealthfolio/ui";
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
    <div className="max-w-5xl space-y-10 pb-6">
      <p className="text-muted-foreground max-w-xl text-[15px]">
        {t(
          "connect:connect.description",
          "Link your brokerage accounts or import data manually. Everything stays on your device.",
        )}
      </p>

      {/* Provider connections are the primary account-ingestion path. */}
      <ProviderConnectionsSection />

      {/* Manual and file-based fallbacks */}
      <section>
        <h3 className={SECTION_HEADING}>
          {t("connect:connect.noAccounts", "Other ways to add accounts")}
        </h3>
        <p className="text-muted-foreground pb-1 text-[15px]">
          {t(
            "connect:connect.noAccountsDescription",
            "Create a manual account or import a file when a direct provider is not available.",
          )}
        </p>
        <Link to="/settings/accounts" state={{ newAccount: true }} className={LIST_ROW}>
          <span className="bg-foreground/[0.07] flex h-10 w-10 shrink-0 items-center justify-center rounded-full">
            <Icons.Plus className="h-[18px] w-[18px]" />
          </span>
          <span className="text-[17px]">
            {t("connect:connect.addAccount", "Add Manual Account")}
          </span>
        </Link>
        <Link to="/import" className={LIST_ROW}>
          <span className="bg-foreground/[0.07] flex h-10 w-10 shrink-0 items-center justify-center rounded-full">
            <Icons.Upload className="h-[18px] w-[18px]" />
          </span>
          <span className="text-[17px]">{t("connect:connect.csvImport", "Import CSV")}</span>
        </Link>
      </section>

      {/* Secure local status */}
      <section className="flex items-start gap-3">
        <span className="bg-foreground/[0.07] flex h-10 w-10 shrink-0 items-center justify-center rounded-full">
          <Icons.Shield className="h-[18px] w-[18px]" />
        </span>
        <div>
          <h4 className="text-[17px] leading-snug">
            {t("connect:connect.secureTitle", "Your Data Stays Local")}
          </h4>
          <p className="text-muted-foreground text-[15px] leading-snug">
            {t(
              "connect:connect.secureDescription",
              "All your financial data is stored securely on your device. Nothing is sent to external servers unless you explicitly export or sync.",
            )}
          </p>
        </div>
      </section>

      {/* Authenticated subject badge */}
      {subject && <p className="text-muted-foreground text-[13px]">Authenticated as: {subject}</p>}
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
