import { useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useDefaultLayout, usePanelRef, type PanelSize } from "react-resizable-panels";
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@/components/ui/resizable";
import { ToastProvider } from "@/components/ui/toast";
import { TooltipProvider } from "@/components/ui/tooltip";
import { ActivityBar } from "@/components/workbench/ActivityBar";
import { CommandPalette } from "@/components/workbench/CommandPalette";
import { Editor } from "@/components/workbench/Editor";
import { SettingsDialog } from "@/components/workbench/SettingsDialog";
import { SideBar } from "@/components/workbench/SideBar";
import { StatusBar } from "@/components/workbench/StatusBar";
import { TitleBar } from "@/components/workbench/TitleBar";
import { Tutor } from "@/components/workbench/Tutor";
import type { OcrStatus } from "@/lib/api";
import { useApp } from "@/lib/store";
import { useZoom } from "@/lib/zoom";

function useBackendEvents() {
  useEffect(() => {
    const app = useApp.getState();
    const subs = [
      listen<{ doc_id: string; done: number; total: number; current: string }>("map-progress", (e) => {
        if (e.payload.doc_id === useApp.getState().doc?.id) app.setMapProgress(e.payload.done, e.payload.total, e.payload.current);
      }),
      listen<string>("doc-changed", (e) => {
        if (e.payload === useApp.getState().doc?.id) {
          app.refreshMap();
          app.refreshNotes();
        }
      }),
      listen<number>("goto-page", (e) => app.goToPage(e.payload)),
      listen<OcrStatus>("ocr-status", (e) => app.setOcrServer(e.payload)),
    ];
    return () => subs.forEach((p) => p.then((un) => un()));
  }, []);
}

function useKeyboard() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const app = useApp.getState();
      if (!e.metaKey || app.settingsOpen) return;
      const k = e.key.toLowerCase();
      const zoom = useZoom.getState();
      const actions: Record<string, () => void> = {
        o: app.choosePdf,
        k: () => app.setPaletteOpen(!app.paletteOpen),
        p: () => app.setPaletteOpen(!app.paletteOpen),
        b: () => (e.altKey ? app.setRightOpen(!app.rightOpen) : app.setLeftOpen(!app.leftOpen)),
        f: () => app.showView("search"),
        l: () => (e.shiftKey ? app.showView("library") : app.focusChat()),
        m: () => e.shiftKey && app.showView("map"),
        n: () => e.shiftKey && app.showView("notes"),
        ",": () => app.openSettings(),
        "=": () => zoom.zoomBy(1.15),
        "+": () => zoom.zoomBy(1.15),
        "-": () => zoom.zoomBy(1 / 1.15),
        "0": () => zoom.fitWidth(),
      };
      // With ⌥ held, macOS reports a different character (e.g. ⌥B is "∫"), so fall back to the key code.
      const key = e.altKey && e.code.startsWith("Key") ? e.code.slice(3).toLowerCase() : k;
      const f = actions[key];
      if (f && !(["m", "n"].includes(key) && !e.shiftKey)) {
        e.preventDefault();
        f();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);
}

/** External links (settings, tutor answers, notes) open in the default browser, not in the app. */
function useExternalLinks() {
  useEffect(() => {
    const onClick = (e: MouseEvent) => {
      const a = (e.target as HTMLElement).closest<HTMLAnchorElement>("a[href]");
      if (!a || !/^https?:/i.test(a.href)) return;
      e.preventDefault();
      openUrl(a.href).catch(console.error);
    };
    document.addEventListener("click", onClick);
    return () => document.removeEventListener("click", onClick);
  }, []);
}

/** Keep the collapsible side panels and the store's open flags in sync, both ways. */
function useSidePanel(open: boolean, setOpen: (o: boolean) => void, getOpen: () => boolean) {
  const ref = usePanelRef();
  const lastWidth = useRef(0);
  useEffect(() => {
    const p = ref.current;
    if (!p) return;
    if (open && p.isCollapsed()) {
      p.expand();
      // expand() can fall back to the minimum size; put back the width it had.
      if (lastWidth.current) p.resize(lastWidth.current);
    }
    if (!open && !p.isCollapsed()) p.collapse();
  }, [open]);
  // Only follow real changes (dragging a panel shut or open), not the first measurement:
  // before layout the group is 0 px wide and every panel reports 0 px.
  const onResize = (size: PanelSize, _id: unknown, prev: PanelSize | undefined) => {
    if (size.inPixels >= 1) lastWidth.current = size.inPixels;
    if (!prev) return;
    const collapsed = size.asPercentage < 0.01;
    if (collapsed !== prev.asPercentage < 0.01 && collapsed === getOpen()) setOpen(!collapsed);
  };
  return { ref, onResize };
}

export default function App() {
  useBackendEvents();
  useKeyboard();
  useExternalLinks();
  useEffect(() => {
    useApp.getState().init();
  }, []);

  const leftOpen = useApp((s) => s.leftOpen);
  const rightOpen = useApp((s) => s.rightOpen);
  const { setLeftOpen, setRightOpen } = useApp.getState();
  const left = useSidePanel(leftOpen, setLeftOpen, () => useApp.getState().leftOpen);
  const right = useSidePanel(rightOpen, setRightOpen, () => useApp.getState().rightOpen);
  const layout = useDefaultLayout({ id: "workbench", storage: localStorage });

  return (
    <TooltipProvider delay={600}>
      <ToastProvider position="bottom-center">
      <div className="grid h-full grid-rows-[30px_minmax(0,1fr)_22px]">
        <TitleBar />
        <div className="flex min-h-0 min-w-0 overflow-hidden">
          <ActivityBar />
          <ResizablePanelGroup orientation="horizontal" id="workbench" defaultLayout={layout.defaultLayout} onLayoutChanged={layout.onLayoutChanged} className="w-0 min-w-0 flex-1">
            <ResizablePanel
              id="side"
              panelRef={left.ref}
              onResize={left.onResize}
              collapsible
              defaultSize={leftOpen ? 280 : 0}
              minSize={200}
              maxSize={520}
              groupResizeBehavior="preserve-pixel-size"
            >
              <SideBar />
            </ResizablePanel>
            <ResizableHandle className="data-[separator=hover]:bg-primary/60 data-[separator=active]:bg-primary" />
            <ResizablePanel id="editor" minSize={320}>
              <Editor />
            </ResizablePanel>
            <ResizableHandle className="data-[separator=hover]:bg-primary/60 data-[separator=active]:bg-primary" />
            <ResizablePanel
              id="tutor"
              panelRef={right.ref}
              onResize={right.onResize}
              collapsible
              defaultSize={rightOpen ? 380 : 0}
              minSize={300}
              maxSize={640}
              groupResizeBehavior="preserve-pixel-size"
            >
              <Tutor />
            </ResizablePanel>
          </ResizablePanelGroup>
        </div>
        <StatusBar />
      </div>
      <SettingsDialog />
      <CommandPalette />
      </ToastProvider>
    </TooltipProvider>
  );
}
