import { useState } from "react";

import { cn } from "@/lib/utils";

/** First letters of the first two words, as a bank app shows an unknown merchant. */
function initials(name: string): string {
  const words = name
    .replace(/^(DL\*|PAYPAL \*|PP\*|SP |PAYFAST\*|YOCO \*)/i, "")
    .split(/[^\p{L}]+/u)
    .filter(Boolean);
  return (
    words.length > 1 ? words[0][0] + words[1][0] : (words[0] ?? "?").slice(0, 2)
  ).toUpperCase();
}

interface MerchantAvatarProps {
  name: string | null;
  currency: string;
  /** Tints the circle behind the initials, usually the category colour. */
  tint?: string | null;
  /** Whether to look for a brand logo; money in and transfers show initials only. */
  logo?: boolean;
  className?: string;
}

/** The merchant's brand logo when the server recognises it, its initials otherwise. */
export function MerchantAvatar({
  name,
  currency,
  tint,
  logo = true,
  className,
}: MerchantAvatarProps) {
  const [failed, setFailed] = useState(false);
  return (
    <span
      className={cn(
        "text-foreground/80 relative flex h-10 w-10 shrink-0 items-center justify-center overflow-hidden rounded-full text-[13px] font-medium",
        className,
      )}
      style={{
        backgroundColor: `color-mix(in srgb, ${tint ?? "var(--foreground)"} ${tint ? 18 : 8}%, transparent)`,
      }}
      aria-hidden="true"
    >
      {name ? initials(name) : ""}
      {name && logo && !failed && (
        <img
          src={`/api/v1/merchant-logo?name=${encodeURIComponent(name)}&currency=${currency}`}
          alt=""
          loading="lazy"
          className="absolute inset-0 h-full w-full object-cover"
          onError={() => setFailed(true)}
        />
      )}
    </span>
  );
}
