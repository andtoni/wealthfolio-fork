import { useFormContext } from "react-hook-form";
import type { AccountSelectOption } from "../forms/fields";
import type { NewActivityFormValues } from "../forms/schemas";
import { MobileActivityTypeStep } from "./mobile-activity-type-step";
import { MobileDetailsStep } from "./mobile-details-step";

interface MobileActivityStepsProps {
  currentStep: number;
  accounts: AccountSelectOption[];
  isEditing: boolean;
  allowTypeChange?: boolean;
}

export function MobileActivitySteps({
  currentStep,
  accounts,
  isEditing,
  allowTypeChange = false,
}: MobileActivityStepsProps) {
  const { watch } = useFormContext<NewActivityFormValues>();
  const activityType = watch("activityType");

  if (isEditing) {
    return (
      <div className="space-y-6">
        {allowTypeChange && <MobileActivityTypeStep />}
        <MobileDetailsStep activityType={activityType} accounts={accounts} isEditing />
      </div>
    );
  }

  return (
    <div className="h-full">
      {currentStep === 1 && <MobileActivityTypeStep />}
      {currentStep === 2 && activityType && (
        <MobileDetailsStep activityType={activityType} accounts={accounts} />
      )}
    </div>
  );
}
