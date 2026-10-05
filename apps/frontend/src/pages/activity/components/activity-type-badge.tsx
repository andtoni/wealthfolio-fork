import { ActivityType, ActivityTypeNames, SUBTYPE_DISPLAY_NAMES } from "@/lib/constants";
import { cn } from "@/lib/utils";

interface ActivityTypeBadgeProps {
  type: ActivityType;
  subtype?: string | null;
  className?: string;
}

function activityTone(type: ActivityType) {
  switch (type) {
    case ActivityType.DIVIDEND:
    case ActivityType.INTEREST:
    case ActivityType.DEPOSIT:
    case ActivityType.TRANSFER_IN:
      return "text-success";
    default:
      return "text-foreground";
  }
}

/** An activity's type as plain text; money coming in is green. */
export function ActivityTypeBadge({ type, subtype, className }: ActivityTypeBadgeProps) {
  const normalizedSubtype = subtype?.trim().toUpperCase();
  const subtypeLabel = normalizedSubtype
    ? (SUBTYPE_DISPLAY_NAMES[normalizedSubtype] ?? subtype)
    : undefined;

  return (
    <span className="inline-flex min-w-0 max-w-full items-center gap-1.5 overflow-hidden">
      <span className={cn("text-sm", activityTone(type), className)}>
        {ActivityTypeNames[type]}
      </span>
      {subtypeLabel && (
        <span className="text-muted-foreground min-w-0 flex-1 truncate text-xs font-normal">
          {subtypeLabel}
        </span>
      )}
    </span>
  );
}
