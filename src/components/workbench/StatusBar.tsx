import { ArrowUpCircle, Check, Cpu, Plug, Sparkles, Square } from "lucide-react";
import { Spinner } from "@/components/ui/spinner";
import { api } from "@/lib/api";
import { useApp } from "@/lib/store";
import { cn } from "@/lib/utils";

const item = "flex items-center gap-1.5 px-2 tabular-nums";
const button = cn(item, "transition-colors hover:bg-accent hover:text-foreground");

export function StatusBar() {
  const settings = useApp((s) => s.settings);
  const task = useApp((s) => s.task);
  const ocrRun = useApp((s) => s.ocrRun);
  const ocrServer = useApp((s) => s.ocrServer);
  const pdf = useApp((s) => s.pdf);
  const page = useApp((s) => s.currentPage);
  const update = useApp((s) => s.update);
  const { openSettings, stopOcr } = useApp.getState();

  let text = task?.text ?? null;
  if (ocrServer.state === "starting") {
    // Installing llama.cpp, downloading or loading the model (from a run or from Settings).
    text = ocrServer.progress !== null ? `${ocrServer.detail} ${Math.round(ocrServer.progress * 100)}%` : `${ocrServer.detail}…`;
  } else if (ocrRun) {
    text = `Recognizing text ${Math.min(ocrRun.done + 1, ocrRun.total)}/${ocrRun.total}`;
  }
  const mcpOk = settings && !settings.mcp_error;

  return (
    <footer className="flex items-stretch justify-between overflow-hidden border-t bg-chrome px-1.5 text-[11px] whitespace-nowrap text-muted-foreground">
      <div className="flex min-w-0 items-stretch">
        <button
          className={button}
          onClick={() => openSettings("mcp-cmd")}
          title={mcpOk ? `Claude Code and Cursor can connect at ${settings.mcp_url}` : `The MCP server isn't running: ${settings?.mcp_error ?? ""}`}
        >
          <Plug className="size-3.5" />
          Agents
          <span className={cn("size-1.5 rounded-full", mcpOk ? "bg-success" : "bg-destructive")} />
        </button>
        {text && (
          <span className={cn(item, "min-w-0")}>
            {task?.done && !ocrRun ? <Check className="size-3.5 text-success" /> : <Spinner className="size-3.5" />}
            <span className="truncate">{text}</span>
            {ocrRun && (
              <button onClick={stopOcr} title="Stop recognizing" className="grid size-[18px] place-items-center rounded-sm hover:bg-accent">
                <Square className="size-3 fill-current" />
              </button>
            )}
          </span>
        )}
      </div>
      <div className="flex items-stretch">
        {update.status.state === "running" ? (
          <button className={button} onClick={() => openSettings("updates")} title="Updating PDF Tutor">
            <Spinner className="size-3.5" />
            {update.status.step}…
          </button>
        ) : (update.check?.behind ?? 0) > 0 || update.status.state === "error" ? (
          <button className={cn(button, "text-primary hover:text-primary")} onClick={() => openSettings("updates")} title="See what's new">
            <ArrowUpCircle className="size-3.5" />
            {update.status.state === "error" ? "Update needs attention" : "Update available"}
          </button>
        ) : null}
        {pdf && <span className={item}>Page {page} of {pdf.numPages}</span>}
        {ocrServer.state === "ready" && (
          <button className={button} onClick={() => api.ocrStop()} title="Qianfan-OCR is in memory (~5 GB) and unloads after 5 idle minutes. Click to unload it now.">
            <Cpu className="size-3.5" />
            OCR model loaded
          </button>
        )}
        {settings && (
          <button className={button} onClick={() => openSettings("provider")} title="Model provider (Settings)">
            <Sparkles className="size-3.5" />
            {settings.provider === "cursor" ? `Cursor · ${settings.cursor_model || "auto"}` : settings.model}
          </button>
        )}
      </div>
    </footer>
  );
}
