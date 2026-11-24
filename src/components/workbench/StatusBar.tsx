import { ArrowUpCircle, Check, Cpu, Plug, Sparkles, Square } from "lucide-react";
import { Spinner } from "@/components/ui/spinner";
import { api, dollars, tokens } from "@/lib/api";
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
  const mapRun = useApp((s) => s.mapRun);
  const { openSettings, stopOcr, stopMap } = useApp.getState();

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
            {mapRun?.tokens != null && (
              <span className="shrink-0 tabular-nums" title="Tokens used by this map build, and the estimated cost at Anthropic's list prices">
                · {tokens(mapRun.tokens)} tokens{mapRun.cost != null && ` · ${dollars(mapRun.cost)}`}
              </span>
            )}
            {mapRun && !mapRun.stopping && (
              <button onClick={stopMap} title="Stop building the map (finished sections are kept)" aria-label="Stop building the map" className="grid size-[18px] shrink-0 place-items-center rounded-sm hover:bg-accent">
                <Square className="size-3 fill-current" />
              </button>
            )}
            {ocrRun && (
              <button onClick={stopOcr} title="Stop recognizing" className="grid size-[18px] place-items-center rounded-sm hover:bg-accent">
                <Square className="size-3 fill-current" />
              </button>
            )}
          </span>
        )}
      </div>
      <div className="flex items-stretch">
        {update.check?.newer && update.dismissed === update.check.latest?.version && update.status.state === "idle" && (
          <button
            className={cn(button, "text-primary hover:text-primary")}
            onClick={() => useApp.setState({ update: { ...useApp.getState().update, dismissed: null } })}
            title="Show the update"
          >
            <ArrowUpCircle className="size-3.5" />
            {update.check.latest?.version} available
          </button>
        )}
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
