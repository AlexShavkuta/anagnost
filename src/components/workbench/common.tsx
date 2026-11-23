import type { ReactNode } from "react";
import { Button, type ButtonProps } from "@/components/ui/button";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";
import { Kbd } from "@/components/ui/kbd";
import { Tooltip, TooltipPopup, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

/** A 24px ghost icon button with a tooltip (and an optional shortcut), for toolbars and headers. */
export function IconButton({
  label,
  shortcut,
  side = "bottom",
  className,
  children,
  ...props
}: ButtonProps & { label: string; shortcut?: string; side?: "top" | "bottom" | "left" | "right" }) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label={label}
            className={cn("text-muted-foreground hover:text-foreground sm:[&_svg:not([class*='size-'])]:size-4", className)}
            {...props}
          />
        }
      >
        {children}
      </TooltipTrigger>
      <TooltipPopup side={side} className="flex items-center gap-2">
        <span className="flex items-center gap-2">
          {label}
          {shortcut && <Kbd className="h-4 text-[10px]">{shortcut}</Kbd>}
        </span>
      </TooltipPopup>
    </Tooltip>
  );
}

/** VS Code style view header: small uppercase title with actions on the right. */
export function ViewHeader({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <header className="flex h-[35px] shrink-0 items-center justify-between pr-2 pl-4">
      <h2 className="text-[11px] font-semibold tracking-[0.06em] text-muted-foreground uppercase">{title}</h2>
      <div className="flex items-center gap-0.5">{children}</div>
    </header>
  );
}

/** coss Empty, sized down for a side bar. */
export function EmptyState({
  icon,
  title,
  children,
  action,
  className,
}: {
  icon?: ReactNode;
  title: string;
  children?: ReactNode;
  action?: ReactNode;
  className?: string;
}) {
  return (
    <Empty className={cn("flex-none gap-3 px-5 py-8 md:py-10", className)}>
      <EmptyHeader>
        {icon && (
          <EmptyMedia variant="icon" className="mb-3 [&_svg]:size-4">
            {icon}
          </EmptyMedia>
        )}
        <EmptyTitle className="text-[13px] font-semibold">{title}</EmptyTitle>
        {children && <EmptyDescription className="text-xs leading-relaxed">{children}</EmptyDescription>}
      </EmptyHeader>
      {action}
    </Empty>
  );
}
