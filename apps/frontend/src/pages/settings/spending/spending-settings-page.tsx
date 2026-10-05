import { useTranslation } from "react-i18next";
import { Link, useNavigate } from "react-router-dom";

import { Button, Icons } from "@wealthfolio/ui";

import { LIST_ROW, SECTION_HEADING } from "@/components/dashboard-card";

import { useSpendingSettings } from "@/features/spending/hooks/use-spending-settings";

import { AccountsCard } from "./components/accounts-card";
import { BudgetCycleCard } from "./components/budget-cycle-card";
import { BudgetOverviewCard } from "./components/budget-overview-card";
import { CategoriesOverviewCard } from "./components/categories-overview-card";
import { EventTypesOverviewCard } from "./components/event-types-overview-card";
import { ModuleCard } from "./components/module-card";
import { RulesOverviewCard } from "./components/rules-overview-card";

export default function SpendingSettingsPage() {
  const { t } = useTranslation();
  const { isEnabled } = useSpendingSettings();
  const navigate = useNavigate();

  return (
    <div className="text-sm leading-relaxed antialiased [&>*+*]:mt-9 [&>section+section]:mt-11">
      <header className="flex items-center gap-1.5 lg:block">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={() => navigate("/settings")}
          className="text-muted-foreground hover:text-foreground -ml-1 h-8 w-8 shrink-0 p-0 lg:hidden"
          aria-label={t("settings:spending.back_to_settings")}
        >
          <Icons.ArrowLeft className="h-5 w-5" />
        </Button>
        <div className="min-w-0 flex-1">
          <h1 className="text-foreground text-2xl font-medium tracking-tight lg:text-[32px]">
            {t("settings:spending.title")}
          </h1>
          <p className="text-muted-foreground mt-1 hidden max-w-[64ch] text-sm sm:block">
            {t("settings:spending.page_description")}
          </p>
        </div>
      </header>

      <ModuleCard />

      {isEnabled && (
        <>
          <Section
            title={t("settings:spending.section_sources")}
            meta={t("settings:spending.section_sources_meta")}
          >
            <AccountsCard />
            {/* CSV import is a common first-run task — surface it next to the
                sources selector so a user landing in Settings to set up the
                tracker can seed historical activities without hunting for the
                separate /import route. Matches the dashboard tab's Import
                action (portfolio-page.tsx). */}
            <Link to="/import" className={LIST_ROW}>
              <span className="bg-foreground/[0.07] flex h-10 w-10 shrink-0 items-center justify-center rounded-full">
                <Icons.Upload className="h-[18px] w-[18px]" aria-hidden />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block text-[17px] leading-snug">
                  {t("settings:spending.import_csv")}
                </span>
                <span className="text-muted-foreground block text-[15px] leading-snug">
                  {t("settings:spending.import_prompt")}
                </span>
              </span>
            </Link>
          </Section>

          <Section
            title={t("settings:spending.section_budgets")}
            meta={t("settings:spending.section_budgets_meta")}
          >
            <BudgetCycleCard />
            <BudgetOverviewCard />
          </Section>

          <Section
            title={t("settings:spending.section_taxonomy")}
            meta={t("settings:spending.section_taxonomy_meta")}
          >
            <div className="grid gap-3 md:grid-cols-3">
              <CategoriesOverviewCard variant="expense" />
              <CategoriesOverviewCard variant="income" />
              <CategoriesOverviewCard variant="savings" />
            </div>
            <EventTypesOverviewCard />
          </Section>

          <Section
            title={t("settings:spending.section_automation")}
            meta={t("settings:spending.section_automation_meta")}
          >
            <RulesOverviewCard />
          </Section>
        </>
      )}
    </div>
  );
}

interface SectionProps {
  title: string;
  meta?: string;
  children: React.ReactNode;
}

function Section({ title, meta, children }: SectionProps) {
  return (
    <section>
      <div className="mb-2 flex items-baseline justify-between gap-3">
        <h2 className={SECTION_HEADING}>{title}</h2>
        {meta && <span className="text-muted-foreground hidden text-sm sm:inline">{meta}</span>}
      </div>
      <div className="space-y-3.5">{children}</div>
    </section>
  );
}
