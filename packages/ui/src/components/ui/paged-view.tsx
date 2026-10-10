import * as React from "react";

import { cn } from "../../lib/utils";

interface PagedViewProps {
  pages: React.ReactNode[];
  className?: string;
}

/**
 * Sections laid out as pages that switch one at a time on a swipe, with page dots underneath.
 *
 * The swipe is the browser's own scrolling with snap points, not a script moving the pages,
 * so it stays smooth however busy the rest of the page is.
 */
export function PagedView({ pages, className }: PagedViewProps) {
  const scrollerRef = React.useRef<HTMLDivElement>(null);
  const [page, setPage] = React.useState(0);

  React.useEffect(() => {
    const scroller = scrollerRef.current;
    if (!scroller) return;
    let frame = 0;
    const onScroll = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        setPage(Math.round(scroller.scrollLeft / Math.max(1, scroller.clientWidth)));
      });
    };
    scroller.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      cancelAnimationFrame(frame);
      scroller.removeEventListener("scroll", onScroll);
    };
  }, []);

  const goTo = (index: number) => {
    const scroller = scrollerRef.current;
    scroller?.scrollTo({ left: index * scroller.clientWidth, behavior: "smooth" });
  };

  return (
    <div className={className}>
      {/* data-no-swipe-drag keeps an enclosing swipable view from taking the gesture. */}
      <div
        ref={scrollerRef}
        data-no-swipe-drag
        className="flex snap-x snap-mandatory overflow-x-auto overscroll-x-contain [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      >
        {pages.map((content, index) => (
          <div key={index} className="min-w-0 shrink-0 grow-0 basis-full snap-start snap-always">
            {content}
          </div>
        ))}
      </div>
      <div className="flex justify-center gap-1.5 pt-4">
        {pages.map((_, index) => (
          <button
            key={index}
            type="button"
            aria-label={`${index + 1} / ${pages.length}`}
            aria-current={index === page}
            onClick={() => goTo(index)}
            className={cn(
              "h-1.5 w-1.5 rounded-full transition-colors",
              index === page ? "bg-foreground" : "bg-foreground/20",
            )}
          />
        ))}
      </div>
    </div>
  );
}
