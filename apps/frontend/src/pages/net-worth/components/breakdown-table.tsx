import { LIST_ROW, SECTION_HEADING } from "@/components/dashboard-card";
import { MerchantAvatar } from "@/features/spending/components/merchant-avatar";
import { useTranslation } from "react-i18next";
import { CompactAmount } from "./compact-amount";
import { CompositionBar } from "./composition-bar";
import {
  CATEGORY_CSS_COLORS,
  deriveChange,
  formatChangePercent,
  seriesFor,
  type Change,
  type ParsedHistoryPoint,
  type ParsedNetWorth,
  type SelectedCategory,
} from "./utils";

function ChangeCell({ change, currency }: { change: Change; currency: string }) {
  const isZero = Math.abs(change.amount) < 0.005;
  const color = isZero
    ? "text-muted-foreground"
    : change.amount > 0
      ? "text-success"
      : "text-destructive";
  const sign = isZero ? "" : change.amount > 0 ? "+" : "-";
  return (
    <div className="flex items-baseline gap-2 text-[13px] tabular-nums">
      <span className={`whitespace-nowrap ${color}`}>
        {sign}
        <CompactAmount value={Math.abs(change.amount)} currency={currency} />
      </span>
      <span className="text-muted-foreground">{formatChangePercent(change.percent)}</span>
    </div>
  );
}

interface RowProps {
  name: string;
  dotColor: string;
  value: number;
  percentOfSection: number;
  change: Change;
  currency: string;
  negative?: boolean;
  onClick?: () => void;
}

function BreakdownRow({
  name,
  dotColor,
  value,
  percentOfSection,
  change,
  currency,
  negative,
  onClick,
}: RowProps) {
  return (
    <div
      className={onClick ? `${LIST_ROW} cursor-pointer` : LIST_ROW}
      onClick={onClick}
      role={onClick ? "button" : undefined}
      tabIndex={onClick ? 0 : undefined}
      onKeyDown={
        onClick
          ? (event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                onClick();
              }
            }
          : undefined
      }
    >
      <MerchantAvatar name={name} currency={currency} tint={dotColor} logo={false} />
      <div className="min-w-0 flex-1">
        <div className="truncate text-[17px] leading-snug">{name}</div>
        <div className="text-muted-foreground text-[15px] tabular-nums leading-snug">
          {percentOfSection.toFixed(1)}%
        </div>
      </div>
      <div className="flex shrink-0 flex-col items-end">
        <span className="text-[19px] tabular-nums leading-snug">
          {negative && value !== 0 ? "-" : ""}
          <CompactAmount value={value} currency={currency} />
        </span>
        <ChangeCell change={change} currency={currency} />
      </div>
    </div>
  );
}

interface BreakdownTableProps {
  data: ParsedNetWorth;
  history: ParsedHistoryPoint[];
  currency: string;
  onSelect: (selected: SelectedCategory) => void;
}

export function BreakdownTable({ data, history, currency, onSelect }: BreakdownTableProps) {
  const { t } = useTranslation();
  const hasLiabilities = data.liabilities.total > 0 || data.liabilities.breakdown.length > 0;

  return (
    <div className="w-full min-w-0">
      <section>
        <div className="flex items-baseline justify-between gap-3 pb-2">
          <h2 className={SECTION_HEADING}>{t("insights:networth.breakdown_table.assets")}</h2>
          <span className="text-muted-foreground text-[17px] tabular-nums">
            <CompactAmount value={data.assets.total} currency={currency} />
          </span>
        </div>
        {/* Composition — proportion of assets (the rows below are its legend) */}
        <CompositionBar data={data} className="mb-2" />
        <div>
          {data.assets.breakdown.map((item) => (
            <BreakdownRow
              key={item.category}
              name={item.name}
              dotColor={CATEGORY_CSS_COLORS[item.category] ?? "var(--muted-foreground)"}
              value={item.value}
              percentOfSection={data.assets.total > 0 ? (item.value / data.assets.total) * 100 : 0}
              change={deriveChange(seriesFor(history, item.category), false)}
              currency={currency}
              onClick={() =>
                onSelect({
                  key: item.category,
                  name: item.name,
                  value: item.value,
                  isLiability: false,
                  isInvestment: item.category === "investments",
                  children: item.children ?? [],
                })
              }
            />
          ))}
        </div>
      </section>

      {hasLiabilities && (
        <section className="pt-8">
          <div className="flex items-baseline justify-between gap-3 pb-2">
            <h2 className={SECTION_HEADING}>
              {t("insights:networth.breakdown_table.liabilities")}
            </h2>
            <span className="text-muted-foreground text-[17px] tabular-nums">
              -<CompactAmount value={data.liabilities.total} currency={currency} />
            </span>
          </div>
          <div>
            {data.liabilities.breakdown.map((item, index) => {
              const key = item.assetId ?? `${item.category}-${index}`;
              const series = item.assetId ? seriesFor(history, item.assetId) : [];
              return (
                <BreakdownRow
                  key={key}
                  name={item.name}
                  dotColor={CATEGORY_CSS_COLORS.liabilities}
                  value={item.value}
                  negative
                  percentOfSection={
                    data.liabilities.total > 0 ? (item.value / data.liabilities.total) * 100 : 0
                  }
                  change={deriveChange(series, true)}
                  currency={currency}
                  onClick={
                    item.assetId
                      ? () =>
                          onSelect({
                            key: item.assetId!,
                            name: item.name,
                            value: item.value,
                            isLiability: true,
                            isInvestment: false,
                            children: [],
                          })
                      : undefined
                  }
                />
              );
            })}
          </div>
        </section>
      )}
    </div>
  );
}
