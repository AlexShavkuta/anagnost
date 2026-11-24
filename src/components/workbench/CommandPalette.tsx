import { ArrowDown, ArrowUp, BookOpenCheck, CornerDownLeft, FileText, FolderOpen, ListTree, NotebookPen, PanelLeft, PanelRight, RefreshCw, ScanText, Square, Search, Settings, Sparkles } from "lucide-react";
import { Fragment, type ReactNode } from "react";
import {
  Command,
  CommandCollection,
  CommandDialog,
  CommandDialogPopup,
  CommandEmpty,
  CommandFooter,
  CommandGroup,
  CommandGroupLabel,
  CommandInput,
  CommandItem,
  CommandList,
  CommandPanel,
  CommandSeparator,
  CommandShortcut,
} from "@/components/ui/command";
import { Kbd, KbdGroup } from "@/components/ui/kbd";
import { useApp } from "@/lib/store";
import { useZoom } from "@/lib/zoom";

interface Item {
  value: string;
  label: string;
  icon: ReactNode;
  hint?: string;
  indent?: number;
  run(): void;
}
interface Group {
  value: string;
  items: Item[];
}

/** ⌘K: jump to a section, run an action, or open a recent PDF. Built on coss Command (Base UI Autocomplete). */
export function CommandPalette() {
  const open = useApp((s) => s.paletteOpen);
  const doc = useApp((s) => s.doc);
  const sections = useApp((s) => s.sections);
  const recent = useApp((s) => s.recent);
  const missing = useApp((s) => s.textStatus.missing.length);
  const app = useApp.getState();

  const groups: Group[] = [];
  if (doc && sections.length) {
    groups.push({
      value: "Sections",
      items: [...sections]
        .sort((a, b) => a.start_page - b.start_page || a.depth - b.depth)
        .map((s) => ({ value: `section-${s.id}`, label: s.title, icon: <ListTree />, hint: `p. ${s.start_page}`, indent: s.depth, run: () => app.goToPage(s.start_page) })),
    });
  }
  const commands: Item[] = [{ value: "open", label: "Open PDF…", icon: <FolderOpen />, hint: "⌘O", run: app.choosePdf }];
  if (doc) {
    commands.push(
      { value: "search", label: "Search the document", icon: <Search />, hint: "⌘F", run: () => app.showView("search") },
      { value: "ask", label: "Ask the tutor", icon: <Sparkles />, hint: "⌘L", run: app.focusChat },
      { value: "quiz", label: "Quiz me on this section", icon: <BookOpenCheck />, run: () => app.sendChat("Quiz me on the section I'm reading. Ask one question at a time, wait for my answer, then give feedback.").catch(() => {}) },
      useApp.getState().mapRun
        ? { value: "stop-map", label: "Stop building the map", icon: <Square />, run: app.stopMap }
        : { value: "map", label: "Build map with Claude", icon: <Sparkles />, run: app.buildMap },
    );
    if (missing > 0) commands.push({ value: "ocr", label: `Recognize text on ${missing} ${missing === 1 ? "page" : "pages"}`, icon: <ScanText />, run: app.recognize });
    commands.push(
      { value: "notes", label: "Show notes", icon: <NotebookPen />, hint: "⇧⌘N", run: () => app.showView("notes") },
      { value: "fit", label: "Fit page width", icon: <FileText />, hint: "⌘0", run: useZoom.getState().fitWidth },
    );
  }
  commands.push(
    { value: "left", label: "Toggle side bar", icon: <PanelLeft />, hint: "⌘B", run: () => app.setLeftOpen(!useApp.getState().leftOpen) },
    { value: "right", label: "Toggle tutor", icon: <PanelRight />, hint: "⌥⌘B", run: () => app.setRightOpen(!useApp.getState().rightOpen) },
    { value: "settings", label: "Settings", icon: <Settings />, hint: "⌘,", run: () => app.openSettings() },
    { value: "update", label: "Check for updates", icon: <RefreshCw />, run: () => { app.openSettings("updates"); app.checkForUpdates(); } },
  );
  groups.push({ value: "Commands", items: commands });
  const others = recent.filter((d) => d.id !== doc?.id);
  if (others.length) {
    groups.push({ value: "Recent", items: others.map((d) => ({ value: `recent-${d.id}`, label: d.title, icon: <FileText />, run: () => app.openPath(d.path) })) });
  }

  const choose = (item: Item) => {
    app.setPaletteOpen(false);
    item.run();
  };

  return (
    <CommandDialog open={open} onOpenChange={(o) => app.setPaletteOpen(o)}>
      <CommandDialogPopup className="max-w-[560px]" aria-label="Command palette">
        <Command items={groups} itemToStringValue={(item: unknown) => (item as Item).label}>
          <CommandInput placeholder={doc ? "Go to a section, or run a command…" : "Run a command…"} className="text-[13px] sm:text-[13px]" />
          <CommandPanel>
            <CommandEmpty>Nothing matches.</CommandEmpty>
            <CommandList className="max-h-[50vh]">
              {(group: Group) => (
                <Fragment key={group.value}>
                  <CommandGroup items={group.items}>
                    <CommandGroupLabel>{group.value}</CommandGroupLabel>
                    <CommandCollection>
                      {(item: Item) => (
                        <CommandItem key={item.value} value={item} onClick={() => choose(item)} className="gap-2 text-[13px] sm:text-[13px] [&_svg]:size-4 [&_svg]:text-muted-foreground">
                          <span className="flex min-w-0 flex-1 items-center gap-2" style={{ paddingLeft: (item.indent ?? 0) * 12 }}>
                            {item.icon}
                            <span className="truncate">{item.label}</span>
                          </span>
                          {item.hint && <CommandShortcut className="tracking-normal tabular-nums">{item.hint}</CommandShortcut>}
                        </CommandItem>
                      )}
                    </CommandCollection>
                  </CommandGroup>
                  <CommandSeparator />
                </Fragment>
              )}
            </CommandList>
          </CommandPanel>
          <CommandFooter className="py-2">
            <div className="flex items-center gap-4">
              <span className="flex items-center gap-2">
                <KbdGroup>
                  <Kbd><ArrowUp /></Kbd>
                  <Kbd><ArrowDown /></Kbd>
                </KbdGroup>
                Navigate
              </span>
              <span className="flex items-center gap-2">
                <Kbd><CornerDownLeft /></Kbd>
                Run
              </span>
            </div>
            <span className="flex items-center gap-2">
              <Kbd>Esc</Kbd>
              Close
            </span>
          </CommandFooter>
        </Command>
      </CommandDialogPopup>
    </CommandDialog>
  );
}
