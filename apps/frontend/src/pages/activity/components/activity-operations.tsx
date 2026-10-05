import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@wealthfolio/ui/components/ui/dropdown-menu";
import { Icons } from "@wealthfolio/ui/components/ui/icons";

import type { Activity, ActivityDetails } from "@/lib/types";
import { getEffectiveType } from "@/lib/types";
import { ActivityType } from "@/lib/constants";
import { useInvestecCardLifecycleLinkEnabled } from "@/lib/provider-features";
import { Row } from "@tanstack/react-table";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ActivityDetailSheet } from "./activity-detail-sheet";

export interface ActivityOperationsProps<TData> {
  row?: Row<TData>;
  activity?: ActivityDetails;
  onEdit: (activity: ActivityDetails) => void | undefined;
  onDelete: (activity: ActivityDetails) => void | undefined;
  onDuplicate: (activity: ActivityDetails) => void | undefined | Promise<void> | Promise<Activity>;
  onLinkTransfer?: (activity: ActivityDetails) => void | undefined;
  onUnlinkTransfer?: (activity: ActivityDetails) => void | undefined;
  onLinkInvestecCardLifecycle?: (activity: ActivityDetails) => void | undefined;
}

export function ActivityOperations<TData>({
  row,
  activity: activityProp,
  onEdit,
  onDelete,
  onDuplicate,
  onLinkTransfer,
  onUnlinkTransfer,
  onLinkInvestecCardLifecycle,
}: ActivityOperationsProps<TData>) {
  const { t } = useTranslation();
  const investecCardLifecycleLinkEnabled = useInvestecCardLifecycleLinkEnabled();
  const activity = activityProp ?? (row?.original as ActivityDetails);
  const [detailSheetOpen, setDetailSheetOpen] = useState(false);
  const effectiveActivityType = getEffectiveType(activity);
  const isTransfer =
    effectiveActivityType === ActivityType.TRANSFER_IN ||
    effectiveActivityType === ActivityType.TRANSFER_OUT;
  const isNew = (activity as ActivityDetails & { isNew?: boolean }).isNew === true;
  const canShowTransferActions = isTransfer && !isNew && (onLinkTransfer || onUnlinkTransfer);
  const canShowInvestecCardLifecycleAction =
    investecCardLifecycleLinkEnabled &&
    !isNew &&
    activity.status === "PENDING" &&
    activity.sourceSystem === "FINANCE" &&
    activity.sourceRecordId?.startsWith("investec-card:") === true &&
    onLinkInvestecCardLifecycle !== undefined;

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger className="hover:bg-foreground/5 text-muted-foreground flex h-8 w-8 shrink-0 items-center justify-center rounded-full transition-colors">
          <Icons.MoreVertical className="h-4 w-4" />
          <span className="sr-only">{t("activity:open_menu")}</span>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onClick={() => setDetailSheetOpen(true)}>
            <Icons.Info className="mr-2 h-4 w-4" />
            {t("activity:operations.more_details")}
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem onClick={() => onEdit(activity)}>
            <Icons.Pencil className="mr-2 h-4 w-4" />
            {t("activity:edit")}
          </DropdownMenuItem>
          <DropdownMenuItem onClick={() => onDuplicate(activity)}>
            <Icons.Copy className="mr-2 h-4 w-4" />
            {t("activity:duplicate")}
          </DropdownMenuItem>
          {canShowTransferActions ? (
            <>
              {activity.sourceGroupId ? (
                onUnlinkTransfer ? (
                  <DropdownMenuItem onClick={() => onUnlinkTransfer(activity)}>
                    <Icons.Unlink className="mr-2 h-4 w-4" />
                    {t("activity:operations.unlink_transfer")}
                  </DropdownMenuItem>
                ) : null
              ) : onLinkTransfer ? (
                <DropdownMenuItem onClick={() => onLinkTransfer(activity)}>
                  <Icons.Link className="mr-2 h-4 w-4" />
                  {t("activity:operations.link_transfer")}
                </DropdownMenuItem>
              ) : null}
            </>
          ) : null}
          {canShowInvestecCardLifecycleAction ? (
            <DropdownMenuItem onClick={() => onLinkInvestecCardLifecycle?.(activity)}>
              <Icons.Link className="mr-2 h-4 w-4" />
              {t("activity:operations.link_card_lifecycle")}
            </DropdownMenuItem>
          ) : null}
          <DropdownMenuSeparator />
          <DropdownMenuItem
            className="text-destructive focus:text-destructive flex cursor-pointer items-center"
            onSelect={() => onDelete(activity)}
          >
            <Icons.Trash className="mr-2 h-4 w-4" />
            {t("activity:delete")}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <ActivityDetailSheet
        activity={activity}
        open={detailSheetOpen}
        onOpenChange={setDetailSheetOpen}
      />
    </>
  );
}
