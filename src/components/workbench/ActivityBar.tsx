import { Library, ListTree, NotebookPen, Search, Settings } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { Kbd } from "@/components/ui/kbd";
import { Tooltip, TooltipPopup, TooltipTrigger } from "@/components/ui/tooltip";
import { useApp, type View } from "@/lib/store";
import { cn } from "@/lib/utils";

const items: { view: View; icon: LucideIcon; label: string; key: string }[] = [
  { view: "map", icon: ListTree, label: "Map", key: "⇧⌘M" },
  { view: "search", icon: Search, label: "Search", key: "⌘F" },
  { view: "notes", icon: NotebookPen, label: "Notes", key: "⇧⌘N" },
  { view: "library", icon: Library, label: "Library", key: "⇧⌘L" },
];

function Item({ icon: Icon, label, shortcut, active, badge, onClick }: {
  icon: LucideIcon; label: string; shortcut: string; active?: boolean; badge?: number; onClick(): void;
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            onClick={onClick}
            aria-label={label}
            aria-current={active ? "page" : undefined}
            className={cn(
              "group relative grid h-12 w-12 place-items-center outline-none focus-visible:bg-accent",
              active && "before:absolute before:inset-y-2.5 before:left-0 before:w-0.5 before:bg-primary",
            )}
          />
        }
      >
        <Icon className={cn("size-[22px] text-muted-foreground transition-colors group-hover:text-foreground", active && "text-foreground")} strokeWidth={1.5} />
        {!!badge && (
          <span className="absolute right-2 bottom-2.5 min-w-4 rounded-full bg-primary px-1 text-center text-[9px] leading-4 font-semibold text-primary-foreground">
            {badge > 99 ? "99+" : badge}
          </span>
        )}
      </TooltipTrigger>
      <TooltipPopup side="right">
        <span className="flex items-center gap-2">
          {label}
          <Kbd className="h-4 text-[10px]">{shortcut}</Kbd>
        </span>
      </TooltipPopup>
    </Tooltip>
  );
}

export function ActivityBar() {
  const view = useApp((s) => s.view);
  const leftOpen = useApp((s) => s.leftOpen);
  const notes = useApp((s) => s.notes.length);
  const { showView, openSettings } = useApp.getState();
  return (
    <nav aria-label="Views" className="flex w-12 shrink-0 flex-col justify-between border-r bg-chrome">
      <div>
        {items.map((it) => (
          <Item
            key={it.view}
            icon={it.icon}
            label={it.label}
            shortcut={it.key}
            active={leftOpen && view === it.view}
            badge={it.view === "notes" ? notes : undefined}
            onClick={() => showView(it.view, true)}
          />
        ))}
      </div>
      <Item icon={Settings} label="Settings" shortcut="⌘," onClick={() => openSettings()} />
    </nav>
  );
}
