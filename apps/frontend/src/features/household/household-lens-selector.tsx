import type { HouseholdLens } from "./household-scope";

interface HouseholdLensSelectorProps {
  enabled: boolean;
  selected: string;
  lenses: HouseholdLens[];
  onSelect(selector: string): void;
}

export function HouseholdLensSelector({
  enabled,
  selected,
  lenses,
  onSelect,
}: HouseholdLensSelectorProps) {
  if (!enabled) return null;

  return (
    <label className="flex items-center gap-2 text-sm">
      <span>Portfolio lens</span>
      <select
        aria-label="Portfolio lens"
        className="border-input bg-background rounded-md border px-2 py-1"
        value={selected}
        onChange={(event) => onSelect(event.target.value)}
      >
        {lenses.map((lens) => (
          <option key={lens.selector} value={lens.selector}>
            {lens.label}
          </option>
        ))}
      </select>
    </label>
  );
}
