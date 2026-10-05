import { LIST_ROW } from "@/components/dashboard-card";
import { TickerAvatar } from "@/components/ticker-avatar";
import { SplitAmount } from "@/features/spending/components/split-amount";
import {
  calculateActivityValue,
  formatSplitRatio,
  isAssetBackedIncomeActivity,
  isCashActivity,
  isCashTransfer,
  isFeeActivity,
  isIncomeActivity,
  isSecuritiesTransfer,
  isSplitActivity,
} from "@/lib/activity-utils";
import { ActivityType, ActivityTypeNames } from "@/lib/constants";
import { parseOccSymbol } from "@/lib/occ-symbol";
import { useSettingsContext } from "@/lib/settings-provider";
import { ActivityDetails } from "@/lib/types";
import { cn, formatDateTime } from "@/lib/utils";
import { Button, EmptyPlaceholder, formatAmount, Icons } from "@wealthfolio/ui";
import { Fragment } from "react";
import { Link } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { ActivityOperations } from "../activity-operations";
import { ActivityTypeBadge } from "../activity-type-badge";

interface ActivityTableMobileProps {
  activities: ActivityDetails[];
  isLoading?: boolean;
  isCompactView: boolean;
  handleEdit: (activity?: ActivityDetails) => void;
  handleDelete: (activity: ActivityDetails) => void;
  onDuplicate: (activity: ActivityDetails) => Promise<void>;
  onLinkTransfer?: (activity: ActivityDetails) => void;
  onUnlinkTransfer?: (activity: ActivityDetails) => void;
  onLinkInvestecCardLifecycle?: (activity: ActivityDetails) => void;
  filtersActive?: boolean;
  onAdd?: () => void;
  onClearFilters?: () => void;
}

export const ActivityTableMobile = ({
  activities,
  isLoading = false,
  isCompactView,
  handleEdit,
  handleDelete,
  onDuplicate,
  onLinkTransfer,
  onUnlinkTransfer,
  onLinkInvestecCardLifecycle,
  filtersActive = false,
  onAdd,
  onClearFilters,
}: ActivityTableMobileProps) => {
  const { t } = useTranslation();
  const { settings } = useSettingsContext();
  const appTimezone = settings?.timezone?.trim() || undefined;

  if (isLoading) {
    return (
      <div className="text-muted-foreground flex h-full items-center justify-center text-sm">
        {t("activity:table.loading")}
      </div>
    );
  }

  if (activities.length === 0) {
    return (
      <EmptyPlaceholder>
        <EmptyPlaceholder.Icon name="Activity" />
        <EmptyPlaceholder.Title>{t("activity:table.no_activities")}</EmptyPlaceholder.Title>
        <EmptyPlaceholder.Description>
          {filtersActive
            ? t("activity:table.no_activities_filtered")
            : t("activity:table.no_activities_desc")}
        </EmptyPlaceholder.Description>
        {filtersActive ? (
          onClearFilters ? (
            <Button variant="outline" onClick={onClearFilters}>
              {t("activity:table.clear_filters")}
            </Button>
          ) : null
        ) : onAdd ? (
          <Button onClick={onAdd}>
            <Icons.Plus className="mr-2 h-4 w-4" aria-hidden="true" />
            {t("activity:add_activity")}
          </Button>
        ) : null}
      </EmptyPlaceholder>
    );
  }

  let lastDay = "";

  return (
    <div className={cn("min-h-0 flex-1 overflow-auto", !isCompactView && "space-y-6")}>
      {activities.map((activity) => {
        const symbol = activity.assetSymbol;
        const activityType = activity.activityType;
        const isTransferActivity =
          activityType === ActivityType.TRANSFER_IN || activityType === ActivityType.TRANSFER_OUT;
        const isAssetBackedIncome = isAssetBackedIncomeActivity(
          activityType,
          symbol,
          activity.assetId,
        );
        const isCash = isTransferActivity
          ? isCashTransfer(activityType, symbol, activity.assetId)
          : isCashActivity(activityType) && !isAssetBackedIncome;
        const hasAsset = Boolean(activity.assetId?.trim());
        const isOptionActivity = activity.instrumentType === "OPTION";
        const parsedOption = isOptionActivity ? parseOccSymbol(symbol) : null;
        const displaySymbol = isCash
          ? t("activity:table.cash")
          : parsedOption
            ? parsedOption.underlying
            : symbol;
        const avatarSymbol = isCash ? "$CASH" : symbol;
        const optionSubtitle = parsedOption
          ? `${new Date(parsedOption.expiration + "T12:00:00").toLocaleDateString("en-US", { month: "short", day: "numeric" })} $${parsedOption.strikePrice} ${parsedOption.optionType}`
          : null;
        const formattedDate = formatDateTime(
          activity.date,
          appTimezone,
          activity.activityDatePrecision,
        );
        const displayValue = calculateActivityValue(activity);

        // Compact View
        if (isCompactView) {
          const heading = formattedDate.date === lastDay ? null : formattedDate.date;
          lastDay = formattedDate.date;
          const showsQuantity =
            !isCash &&
            !(isIncomeActivity(activity.activityType) && !isAssetBackedIncome) &&
            !isSplitActivity(activity.activityType) &&
            !isFeeActivity(activity.activityType) &&
            activity.quantity;
          const inner = (
            <>
              <TickerAvatar symbol={avatarSymbol} className="h-10 w-10 shrink-0" />
              <div className="min-w-0 flex-1">
                <p className="truncate text-[17px] leading-snug">{displaySymbol}</p>
                <p className="text-muted-foreground flex gap-1.5 truncate text-[15px] leading-snug">
                  <span>{ActivityTypeNames[activity.activityType]}</span>
                  {optionSubtitle && <span>{optionSubtitle}</span>}
                  {showsQuantity && (
                    <span>
                      {activity.quantity}{" "}
                      {isOptionActivity
                        ? t("activity:date_list.contracts")
                        : t("activity:date_list.shares")}
                    </span>
                  )}
                </p>
              </div>
              {activity.activityType !== "SPLIT" && (
                <span className="shrink-0 text-[19px]">
                  <SplitAmount value={displayValue} currency={activity.currency} />
                </span>
              )}
            </>
          );
          return (
            <Fragment key={activity.id}>
              {heading && (
                <h3 className="pb-1 pt-6 text-xl font-medium tracking-tight first:pt-0">
                  {heading}
                </h3>
              )}
              <div className={LIST_ROW}>
                {isCash || !hasAsset ? (
                  <div className="flex min-w-0 flex-1 items-center gap-3">{inner}</div>
                ) : (
                  <Link
                    to={`/holdings/${encodeURIComponent(activity.assetId)}`}
                    className="flex min-w-0 flex-1 items-center gap-3"
                  >
                    {inner}
                  </Link>
                )}
                <ActivityOperations
                  activity={activity}
                  onEdit={handleEdit}
                  onDelete={handleDelete}
                  onDuplicate={onDuplicate}
                  onLinkTransfer={onLinkTransfer}
                  onUnlinkTransfer={onUnlinkTransfer}
                  onLinkInvestecCardLifecycle={onLinkInvestecCardLifecycle}
                />
              </div>
            </Fragment>
          );
        }

        // Detailed View
        return (
          <div key={activity.id}>
            <div className="space-y-2">
              {/* Header: Symbol and Date */}
              <div className="flex items-start justify-between">
                {(() => {
                  const inner = (
                    <>
                      <TickerAvatar symbol={avatarSymbol} className="h-10 w-10" />
                      <div>
                        <p className="text-[17px] leading-snug">{displaySymbol}</p>
                        <p className="text-muted-foreground text-xs">
                          {isCash ? activity.currency : (optionSubtitle ?? activity.assetName)}
                        </p>
                      </div>
                    </>
                  );
                  return isCash || !hasAsset ? (
                    <div className="flex items-center gap-2">{inner}</div>
                  ) : (
                    <Link
                      to={`/holdings/${encodeURIComponent(activity.assetId)}`}
                      className="flex items-center gap-2"
                    >
                      {inner}
                    </Link>
                  );
                })()}
                <ActivityOperations
                  activity={activity}
                  onEdit={handleEdit}
                  onDelete={handleDelete}
                  onDuplicate={onDuplicate}
                  onLinkTransfer={onLinkTransfer}
                  onUnlinkTransfer={onUnlinkTransfer}
                  onLinkInvestecCardLifecycle={onLinkInvestecCardLifecycle}
                />
              </div>

              {/* Activity Details Grid */}
              <div className="space-y-1.5 text-sm">
                {/* Date and Type */}
                <div className="flex items-center justify-between">
                  <span className="text-muted-foreground">{t("activity:table_date")}</span>
                  <div className="text-right">
                    <p>{formattedDate.date}</p>
                    <p className="text-muted-foreground text-xs">{formattedDate.time}</p>
                  </div>
                </div>

                <div className="flex items-center justify-between">
                  <span className="text-muted-foreground">{t("activity:table_type")}</span>
                  <ActivityTypeBadge
                    type={activity.activityType}
                    subtype={activity.subtype}
                    className="text-xs font-normal"
                  />
                </div>

                {/* Quantity (if applicable) */}
                {!isCash &&
                  !(isIncomeActivity(activity.activityType) && !isAssetBackedIncome) &&
                  !isSplitActivity(activity.activityType) &&
                  !isFeeActivity(activity.activityType) &&
                  activity.quantity && (
                    <div className="flex items-center justify-between">
                      <span className="text-muted-foreground">
                        {isOptionActivity
                          ? t("activity:detail.contracts")
                          : t("activity:field_shares")}
                      </span>
                      <span className="font-medium">{activity.quantity}</span>
                    </div>
                  )}

                {/* Price/Amount */}
                <div className="flex items-center justify-between">
                  <span className="text-muted-foreground">
                    {activity.activityType === "SPLIT"
                      ? t("activity:table.ratio")
                      : (isCashActivity(activity.activityType) &&
                            !isAssetBackedIncome &&
                            !isSecuritiesTransfer(
                              activity.activityType,
                              symbol,
                              activity.assetId,
                            )) ||
                          isCashTransfer(activity.activityType, symbol, activity.assetId) ||
                          (isIncomeActivity(activity.activityType) && !isAssetBackedIncome)
                        ? t("activity:form.label_amount")
                        : isOptionActivity
                          ? t("activity:table.premium")
                          : t("activity:field_price")}
                  </span>
                  <span className="font-medium">
                    {activity.activityType === "FEE"
                      ? "-"
                      : activity.activityType === "SPLIT"
                        ? formatSplitRatio(Number(activity.amount))
                        : (isCashActivity(activity.activityType) &&
                              !isAssetBackedIncome &&
                              !isSecuritiesTransfer(
                                activity.activityType,
                                symbol,
                                activity.assetId,
                              )) ||
                            isCashTransfer(activity.activityType, symbol, activity.assetId) ||
                            (isIncomeActivity(activity.activityType) && !isAssetBackedIncome)
                          ? formatAmount(Number(activity.amount), activity.currency)
                          : formatAmount(Number(activity.unitPrice), activity.currency)}
                  </span>
                </div>

                {/* Fee (if applicable) */}
                {Number(activity.fee) > 0 && activity.activityType !== "SPLIT" && (
                  <div className="flex items-center justify-between">
                    <span className="text-muted-foreground">{t("activity:table_fee")}</span>
                    <span className="font-medium">
                      {formatAmount(Number(activity.fee), activity.currency)}
                    </span>
                  </div>
                )}
                {Number(activity.tax) > 0 && activity.activityType !== "SPLIT" && (
                  <div className="flex items-center justify-between">
                    <span className="text-muted-foreground">{t("activity:table.tax")}</span>
                    <span className="font-medium">
                      {formatAmount(Number(activity.tax), activity.currency)}
                    </span>
                  </div>
                )}

                {/* Total Value */}
                {activity.activityType !== "SPLIT" && (
                  <div className="flex items-center justify-between">
                    <span className="text-muted-foreground font-medium">
                      {t("activity:table.total_value")}
                    </span>
                    <span className="font-medium">
                      {formatAmount(displayValue, activity.currency)}
                    </span>
                  </div>
                )}

                {/* Account */}
                <div className="flex items-center justify-between">
                  <span className="text-muted-foreground">{t("activity:table_account")}</span>
                  <div className="text-right">
                    <p>{activity.accountName}</p>
                    <p className="text-muted-foreground text-xs">{activity.accountCurrency}</p>
                  </div>
                </div>
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
};

export default ActivityTableMobile;
