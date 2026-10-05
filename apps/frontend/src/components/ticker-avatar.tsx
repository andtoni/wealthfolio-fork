import { useEffect, useState } from "react";

import { cn } from "@/lib/utils";
import { parseOccSymbol } from "@/lib/occ-symbol";
import { Avatar, AvatarFallback, AvatarImage } from "@wealthfolio/ui";

interface TickerAvatarProps {
  symbol: string;
  className?: string;
  imageClassName?: string;
}

const CASH_AVATAR_LABELS: Record<string, string> = {
  USD: "$",
  CAD: "C$",
  AUD: "A$",
  NZD: "NZ$",
};

const CASH_SYMBOL_PATTERN = /^\$?CASH[-_:]([A-Z]{3})$/;

const getCashAvatarLabel = (symbol: string): string | null => {
  const normalized = symbol.trim().toUpperCase();
  if (normalized === "$CASH" || normalized === "CASH") return "$";

  const currency = CASH_SYMBOL_PATTERN.exec(normalized)?.[1];
  if (!currency) return null;

  return CASH_AVATAR_LABELS[currency] ?? currency;
};

const getFallbackAvatarLabel = (symbol: string): string => symbol.slice(0, 4);

export const TickerAvatar = ({
  symbol,
  className = "size-8",
  imageClassName = "object-contain p-2",
}: TickerAvatarProps) => {
  // For OCC option symbols (e.g. "AAPL250321C00150000"), use the underlying ticker for logo
  const parsed = symbol ? parseOccSymbol(symbol) : null;
  const logoSymbol = parsed ? parsed.underlying : symbol;

  // Extract the base symbol (before any dot, hyphen, or colon) for fallback
  const baseSymbol = logoSymbol ? logoSymbol.split(/[.:-]/)[0].toUpperCase() : "";
  const fullSymbol = logoSymbol ? logoSymbol.toUpperCase() : "";

  // Try full symbol first, then fallback to base symbol
  const primaryLogoUrl = fullSymbol ? `/ticker-logos/${fullSymbol}.png` : "";
  const fallbackLogoUrl = baseSymbol ? `/ticker-logos/${baseSymbol}.png` : "";
  const cashAvatarLabel = getCashAvatarLabel(fullSymbol);
  const fallbackAvatarLabel = baseSymbol ? getFallbackAvatarLabel(baseSymbol) : "•";
  const [logoUrl, setLogoUrl] = useState(primaryLogoUrl);

  useEffect(() => {
    setLogoUrl(primaryLogoUrl);
  }, [primaryLogoUrl]);

  if (cashAvatarLabel) {
    return (
      <Avatar className={cn("border-white/20 font-medium backdrop-blur-md", className)}>
        <AvatarFallback className="bg-primary/80 dark:bg-primary/20 text-xs font-medium text-white">
          <span className="p-1" title={fullSymbol}>
            {cashAvatarLabel}
          </span>
        </AvatarFallback>
      </Avatar>
    );
  }

  return (
    <Avatar
      className={cn("bg-primary/80 dark:bg-primary/20 border-white/20 backdrop-blur-md", className)}
    >
      <AvatarImage
        src={logoUrl}
        alt={fullSymbol}
        className={imageClassName}
        onLoadingStatusChange={(status) => {
          if (
            status === "error" &&
            logoUrl === primaryLogoUrl &&
            fallbackLogoUrl !== primaryLogoUrl
          ) {
            setLogoUrl(fallbackLogoUrl);
          }
        }}
      />
      <AvatarFallback className="bg-primary/80 dark:bg-primary/20 font-medium text-white">
        <span
          className={cn(
            "px-0.5 leading-none",
            fallbackAvatarLabel.length >= 4 ? "text-[10px]" : "text-xs",
          )}
          title={fullSymbol}
        >
          {fallbackAvatarLabel}
        </span>
      </AvatarFallback>
    </Avatar>
  );
};
