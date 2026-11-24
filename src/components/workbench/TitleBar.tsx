import { PanelLeft, PanelRight, Search } from "lucide-react";
import { Kbd } from "@/components/ui/kbd";
import { useApp } from "@/lib/store";
import { cn } from "@/lib/utils";
import { IconButton } from "./common";

export function TitleBar() {
  const doc = useApp((s) => s.doc);
  const leftOpen = useApp((s) => s.leftOpen);
  const rightOpen = useApp((s) => s.rightOpen);
  const { setLeftOpen, setRightOpen, setPaletteOpen } = useApp.getState();

  return (
    <header data-tauri-drag-region className="grid grid-cols-[1fr_minmax(0,460px)_1fr] items-center border-b bg-chrome">
      <div data-tauri-drag-region className="h-full" />
      <button
        onClick={() => setPaletteOpen(true)}
        className="relative flex h-[22px] items-center justify-center gap-1.5 overflow-hidden rounded-md border border-input bg-background px-2 text-xs whitespace-nowrap text-muted-foreground shadow-xs/5 transition-colors hover:text-foreground dark:bg-input/32"
      >
        <Search className="size-3.5 shrink-0 opacity-80" />
        <span className="truncate">{doc?.title ?? "Anagnost"}</span>
        <Kbd className="absolute right-1 h-4 text-[10px]">⌘K</Kbd>
      </button>
      <div data-tauri-drag-region className="flex h-full items-center justify-end gap-0.5 pr-2">
        <IconButton label="Toggle Side Bar" shortcut="⌘B" onClick={() => setLeftOpen(!leftOpen)} aria-pressed={leftOpen}>
          <PanelLeft className={cn(!leftOpen && "opacity-50")} />
        </IconButton>
        <IconButton label="Toggle Tutor" shortcut="⌥⌘B" onClick={() => setRightOpen(!rightOpen)} aria-pressed={rightOpen}>
          <PanelRight className={cn(!rightOpen && "opacity-50")} />
        </IconButton>
      </div>
    </header>
  );
}
