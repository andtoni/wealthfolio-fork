import { IngestionHubContent } from "@/features/account-ingestion/pages/account-ingestion-page";
import { Separator } from "@wealthfolio/ui/components/ui/separator";
import { useTranslation } from "react-i18next";
import { SettingsHeader } from "../settings-header";

export default function ConnectSettingsPage() {
  const { t } = useTranslation();

  return (
    <div className="space-y-6">
      <SettingsHeader
        heading={t("settings:nav.items.connect")}
        text={t("settings:connect_description")}
      />
      <Separator />
      <IngestionHubContent />
    </div>
  );
}
