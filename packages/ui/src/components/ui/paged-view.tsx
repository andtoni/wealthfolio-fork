import useEmblaCarousel from "embla-carousel-react";
import * as React from "react";

import { cn } from "../../lib/utils";

interface PagedViewProps {
  pages: React.ReactNode[];
  className?: string;
}

/** Sections laid out as pages that switch one at a time on a swipe, with page dots underneath. */
export function PagedView({ pages, className }: PagedViewProps) {
  const [emblaRef, emblaApi] = useEmblaCarousel({ align: "start" });
  const [page, setPage] = React.useState(0);

  React.useEffect(() => {
    if (!emblaApi) return;
    const onSelect = () => setPage(emblaApi.selectedScrollSnap());
    emblaApi.on("select", onSelect);
    return () => {
      emblaApi.off("select", onSelect);
    };
  }, [emblaApi]);

  return (
    <div className={className}>
      {/* data-no-swipe-drag keeps an enclosing swipable view from taking the gesture. */}
      <div ref={emblaRef} data-no-swipe-drag className="overflow-hidden">
        <div className="flex touch-pan-y">
          {pages.map((content, index) => (
            <div key={index} className="min-w-0 shrink-0 grow-0 basis-full">
              {content}
            </div>
          ))}
        </div>
      </div>
      <div className="flex justify-center gap-1.5 pt-4">
        {pages.map((_, index) => (
          <button
            key={index}
            type="button"
            aria-label={`${index + 1} / ${pages.length}`}
            aria-current={index === page}
            onClick={() => emblaApi?.scrollTo(index)}
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
