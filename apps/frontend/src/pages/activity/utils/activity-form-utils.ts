import { ActivityType } from "@/lib/constants";
import type { ActivityDetails } from "@/lib/types";
import type { PickerActivityType } from "../config/activity-form-config";

const PURE_CASH_ACTIVITY_TYPES: readonly string[] = [
  ActivityType.DEPOSIT,
  ActivityType.WITHDRAWAL,
  ActivityType.FEE,
  ActivityType.INTEREST,
  ActivityType.TAX,
];

/**
 * Maps a database activity type to the picker activity type.
 * TRANSFER_IN/TRANSFER_OUT are merged into TRANSFER for the picker UI.
 */
export function mapActivityTypeToPicker(
  activityType?: string | null,
): PickerActivityType | undefined {
  if (!activityType) return undefined;
  if (activityType === ActivityType.TRANSFER_IN || activityType === ActivityType.TRANSFER_OUT) {
    return "TRANSFER";
  }
  return activityType as PickerActivityType;
}

/**
 * Checks if the activity type is a pure cash activity (no asset involved).
 * Used to determine if account currency should be included in payload.
 */
export function isPureCashActivity(activityType: string): boolean {
  return PURE_CASH_ACTIVITY_TYPES.includes(activityType);
}

/**
 * A standalone activity can change type while editing. Paired transfers must
 * stay in the paired-transfer editor so one leg cannot silently diverge from
 * its counterpart.
 */
export function canChangeActivityType(
  activity: Partial<ActivityDetails> | undefined,
  hidePicker = false,
): boolean {
  const activityType = activity?.activityTypeOverride ?? activity?.activityType;
  const isTransfer =
    activityType === ActivityType.TRANSFER_IN || activityType === ActivityType.TRANSFER_OUT;
  return Boolean(
    activity?.id && isTransfer && !hidePicker && !activity.transferOutId && !activity.transferInId,
  );
}
