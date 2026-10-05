import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import * as React from "react";

import { cn } from "../../lib/utils";

// Information reads as plain muted text; only a state that needs attention
// (success, warning, destructive) gets a small tinted tag.
const badgeVariants = cva(
  "inline-flex items-center justify-center text-xs w-fit whitespace-nowrap shrink-0 [&>svg]:size-3 gap-1 [&>svg]:pointer-events-none overflow-hidden",
  {
    variants: {
      variant: {
        default: "text-muted-foreground",
        secondary: "text-muted-foreground",
        outline: "text-muted-foreground",
        info: "text-muted-foreground",
        destructive: "rounded-md bg-destructive/10 px-1.5 py-0.5 text-destructive",
        success: "rounded-md bg-success/15 px-1.5 py-0.5 text-success",
        warning: "rounded-md bg-warning/15 px-1.5 py-0.5 text-warning",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  },
);

function Badge({
  className,
  variant,
  asChild = false,
  ...props
}: React.ComponentProps<"span"> & VariantProps<typeof badgeVariants> & { asChild?: boolean }) {
  const Comp = asChild ? Slot : "span";

  return <Comp data-slot="badge" className={cn(badgeVariants({ variant }), className)} {...props} />;
}

export { Badge, badgeVariants };
