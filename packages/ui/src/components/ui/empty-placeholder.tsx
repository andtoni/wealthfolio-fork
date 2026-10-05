import * as React from "react";

import { Icons } from "./icons";
import { cn } from "../../lib/utils";

interface EmptyPlaceholderProps extends React.HTMLAttributes<HTMLDivElement> {
  icon?: React.ReactNode;
  title?: string;
  description?: string;
}

export function EmptyPlaceholder({ className, children, icon, title, description, ...props }: EmptyPlaceholderProps) {
  return (
    <div
      className={cn(
        "animate-in fade-in-50 flex flex-col items-center justify-center rounded-md p-8 text-center",
        className,
      )}
      {...props}
    >
      <div className="mx-auto flex max-w-[420px] flex-col items-center justify-center text-center">
        {icon && (
          <div className="bg-foreground/[0.07] flex size-16 items-center justify-center rounded-full">{icon}</div>
        )}
        {title && <EmptyPlaceholder.Title>{title}</EmptyPlaceholder.Title>}
        {description && <EmptyPlaceholder.Description>{description}</EmptyPlaceholder.Description>}
        {children}
      </div>
    </div>
  );
}

interface EmptyPlaceholderIconProps extends Partial<React.SVGProps<SVGSVGElement>> {
  name: keyof typeof Icons;
}

EmptyPlaceholder.Icon = function EmptyPlaceHolderIcon({ name, className, ...props }: EmptyPlaceholderIconProps) {
  const Icon = Icons[name];

  if (!Icon) {
    return null;
  }

  // Filter out problematic props that might cause type issues
  const { children: _children, dominantBaseline: _dominantBaseline, ...validProps } = props;

  return (
    <div className="bg-foreground/[0.07] flex h-16 w-16 items-center justify-center rounded-full">
      <Icon className={cn("h-7 w-7", className)} {...validProps} />
    </div>
  );
};

type EmptyPlacholderTitleProps = React.HTMLAttributes<HTMLHeadingElement>;

EmptyPlaceholder.Title = function EmptyPlaceholderTitle({ className, ...props }: EmptyPlacholderTitleProps) {
  return <h2 className={cn("mt-5 text-xl font-medium tracking-tight", className)} {...props} />;
};

type EmptyPlacholderDescriptionProps = React.HTMLAttributes<HTMLParagraphElement>;

EmptyPlaceholder.Description = function EmptyPlaceholderDescription({
  className,
  ...props
}: EmptyPlacholderDescriptionProps) {
  return (
    <p
      className={cn("text-muted-foreground mb-6 mt-1.5 text-center text-[15px] font-normal leading-snug", className)}
      {...props}
    />
  );
};
