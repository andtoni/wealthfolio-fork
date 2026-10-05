import { getGoals } from "@/adapters";
import { DashboardCard, LIST_ROW } from "@/components/dashboard-card";
import { Icons } from "@wealthfolio/ui/components/ui/icons";
import { Skeleton } from "@wealthfolio/ui/components/ui/skeleton";
import { useBalancePrivacy } from "@/hooks/use-balance-privacy";
import { QueryKeys } from "@/lib/query-keys";
import type { Goal } from "@/lib/types";
import { useQuery } from "@tanstack/react-query";
import { cn, formatCompactAmount } from "@wealthfolio/ui";
import { useTranslation } from "react-i18next";
import { Link } from "react-router-dom";

const MAX_DISPLAYED_GOALS = 5;

function goalTime(goal: Goal) {
  const date = goal.targetDate ?? goal.projectedCompletionDate;
  if (!date) return Infinity;
  const time = new Date(date).getTime();
  return Number.isFinite(time) ? time : Infinity;
}

function goalTarget(goal: Goal) {
  return goal.summaryTargetAmount ?? goal.targetAmount ?? 0;
}

function sortDashboardGoals(a: Goal, b: Goal) {
  const aRetirement = a.goalType === "retirement" ? 0 : 1;
  const bRetirement = b.goalType === "retirement" ? 0 : 1;
  if (aRetirement !== bRetirement) return aRetirement - bRetirement;

  const dateDiff = goalTime(a) - goalTime(b);
  if (dateDiff !== 0) return dateDiff;

  const targetDiff = goalTarget(b) - goalTarget(a);
  if (targetDiff !== 0) return targetDiff;

  return new Date(a.createdAt).getTime() - new Date(b.createdAt).getTime();
}

function statusRingClass(status: Goal["statusHealth"]) {
  if (status === "off_track") return "text-destructive";
  if (status === "at_risk") return "text-amber-500";
  if (status === "on_track") return "text-success";
  return "text-muted-foreground";
}

const RING_RADIUS = 17;
const RING_LENGTH = 2 * Math.PI * RING_RADIUS;

/** Progress towards a goal as a ring, in the place a row's avatar takes. */
function ProgressRing({ fraction, className }: { fraction: number; className: string }) {
  return (
    <svg viewBox="0 0 40 40" className={cn("h-10 w-10 shrink-0 -rotate-90", className)} aria-hidden>
      <circle
        cx="20"
        cy="20"
        r={RING_RADIUS}
        fill="none"
        strokeWidth="3"
        className="stroke-foreground/10"
      />
      <circle
        cx="20"
        cy="20"
        r={RING_RADIUS}
        fill="none"
        strokeWidth="3"
        strokeLinecap="round"
        stroke="currentColor"
        strokeDasharray={`${fraction * RING_LENGTH} ${RING_LENGTH}`}
      />
    </svg>
  );
}

function formatTimeRemaining(t: (key: string) => string, targetDate?: string): string {
  if (!targetDate) return t("dashboard:goals.time_no_deadline");
  const target = new Date(targetDate);
  const now = new Date();
  if (!Number.isFinite(target.getTime())) return t("dashboard:goals.time_no_deadline");
  if (target.getTime() <= now.getTime()) return t("dashboard:goals.time_due");
  let months =
    (target.getFullYear() - now.getFullYear()) * 12 + (target.getMonth() - now.getMonth());
  if (target.getDate() < now.getDate()) months -= 1;
  if (months < 0) months = 0;
  const years = Math.floor(months / 12);
  const remMonths = months % 12;
  // Compact, language-neutral units (Y/M).
  if (years === 0) return `${Math.max(1, remMonths)}M`;
  if (remMonths === 0) return `${years}Y`;
  return `${years}Y ${remMonths}M`;
}

function ViewAllLink() {
  const { t } = useTranslation();
  return (
    <Link
      to="/goals"
      className="text-muted-foreground hover:bg-foreground/5 inline-flex h-8 items-center rounded-md px-3 text-sm transition-colors"
    >
      {t("dashboard:view_all")}
      <Icons.ChevronRight className="ml-1 h-3 w-3" />
    </Link>
  );
}

export function SavingGoals() {
  const { t } = useTranslation();
  const { isBalanceHidden } = useBalancePrivacy();

  const { data: goals, isLoading } = useQuery<Goal[], Error>({
    queryKey: [QueryKeys.GOALS],
    queryFn: getGoals,
  });

  if (isLoading) {
    return (
      <DashboardCard title={t("common:goals")}>
        <div className="space-y-6">
          {[0, 1, 2].map((i) => (
            <div key={i} className="space-y-2">
              <Skeleton className="h-4 w-1/3" />
              <Skeleton className="h-[2px] w-2/3" />
              <Skeleton className="h-3 w-1/4" />
            </div>
          ))}
        </div>
      </DashboardCard>
    );
  }

  const activeGoals = (goals ?? [])
    .filter((g) => g.statusLifecycle === "active")
    .sort(sortDashboardGoals);
  const visibleGoals = activeGoals.slice(0, MAX_DISPLAYED_GOALS);
  const hiddenGoalsCount = Math.max(0, activeGoals.length - visibleGoals.length);

  if (activeGoals.length === 0) {
    return (
      <DashboardCard title={t("common:goals")} action={<ViewAllLink />}>
        <p className="text-muted-foreground text-[15px]">
          {t("dashboard:goals.no_goals_set")}{" "}
          <Link to="/goals/new" className="text-foreground hover:underline">
            {t("dashboard:goals.create_first_goal")}
          </Link>
        </p>
      </DashboardCard>
    );
  }

  return (
    <DashboardCard title={t("common:goals")} action={<ViewAllLink />}>
      {visibleGoals.map((goal) => {
        const progress = goal.summaryProgress ?? 0;
        const pct = Math.max(0, Math.min(1, progress));
        const currentValue = goal.summaryCurrentValue ?? 0;
        const target = goal.summaryTargetAmount ?? goal.targetAmount ?? 0;
        const currency = goal.currency ?? "USD";
        const deadline = goal.targetDate ?? goal.projectedCompletionDate;
        const timeStr = formatTimeRemaining(t, deadline);
        const pctDisplay = Math.round(pct * 100);

        const currentDisplay = isBalanceHidden
          ? "••••"
          : formatCompactAmount(currentValue, currency);
        const targetDisplay = isBalanceHidden
          ? "••••"
          : target > 0
            ? formatCompactAmount(target, currency)
            : "—";

        return (
          <Link key={goal.id} to={`/goals/${goal.id}`} className={LIST_ROW}>
            <ProgressRing fraction={pct} className={statusRingClass(goal.statusHealth)} />
            <div className="min-w-0 flex-1">
              <h3 className="truncate text-[17px] leading-snug">{goal.title}</h3>
              <div className="text-muted-foreground text-[15px] tabular-nums leading-snug">
                {currentDisplay} / {targetDisplay}
              </div>
            </div>
            <div className="shrink-0 text-right">
              <div className="text-[19px] tabular-nums leading-snug">{pctDisplay}%</div>
              <div className="text-muted-foreground text-[13px]">{timeStr}</div>
            </div>
          </Link>
        );
      })}
      {hiddenGoalsCount > 0 && (
        <Link
          to="/goals"
          className="text-muted-foreground hover:text-foreground flex items-center justify-between py-2.5 text-[15px] transition-colors"
        >
          <span>{t("dashboard:goals.more_goals", { count: hiddenGoalsCount })}</span>
          <Icons.ChevronRight className="h-4 w-4" />
        </Link>
      )}
    </DashboardCard>
  );
}

export default SavingGoals;
