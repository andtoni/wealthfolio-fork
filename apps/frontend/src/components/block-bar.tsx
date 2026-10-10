import { Link } from "react-router-dom";

import { cn } from "@/lib/utils";

export interface BlockBarSegment {
  value: number;
  color: string;
  /** Shown when the pointer rests on the block. */
  label?: string;
  /** Makes the block a link. */
  href?: string;
}

interface BlockBarProps {
  segments: BlockBarSegment[];
  /** What the whole bar stands for. Defaults to the sum; a larger total leaves an empty block. */
  total?: number;
  className?: string;
}

/** Shares drawn as thick rounded blocks with gaps between them. */
export function BlockBar({ segments, total, className }: BlockBarProps) {
  const shown = segments.filter((segment) => segment.value > 0);
  const sum = shown.reduce((acc, segment) => acc + segment.value, 0);
  const rest = Math.max(total ?? sum, sum) - sum;

  return (
    <div className={cn("flex h-3.5 w-full gap-1", className)}>
      {shown.map((segment, index) => {
        const style = { flex: `${segment.value} 0 0`, backgroundColor: segment.color };
        return segment.href ? (
          // The bar is thin, so the link reaches a little above and below it.
          <Link
            key={index}
            to={segment.href}
            className="relative min-w-1.5 rounded-[5px] after:absolute after:-inset-y-3 after:inset-x-0"
            style={style}
            title={segment.label}
            aria-label={segment.label}
          />
        ) : (
          <div
            key={index}
            className="min-w-1.5 rounded-[5px]"
            style={style}
            title={segment.label}
          />
        );
      })}
      {(rest > 0 || shown.length === 0) && (
        <div className="bg-foreground/10 rounded-[5px]" style={{ flex: `${rest || 1} 0 0` }} />
      )}
    </div>
  );
}
