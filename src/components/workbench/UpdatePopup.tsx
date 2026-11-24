// Bottom-right card that announces a new release and installs it.
// Non-modal: reading continues underneath. "Later" hides it until the next launch.

import { ArrowUpCircle, X } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Button } from "@/components/ui/button";
import { Progress, ProgressIndicator, ProgressTrack } from "@/components/ui/progress";
import { Spinner } from "@/components/ui/spinner";
import { useApp } from "@/lib/store";

export function UpdatePopup() {
  const update = useApp((s) => s.update);
  const { startUpdate, dismissUpdate } = useApp.getState();
  const { check, status, dismissed } = update;
  const latest = check?.latest;
  const running = status.state === "running";
  const failed = status.state === "error";
  if (!check?.newer || !latest || (dismissed === latest.version && !running && !failed)) return null;

  const notes = latest.notes.slice(0, 3);
  const more = latest.notes.length - notes.length;
  const canInstall = latest.downloadable && !check.unavailable;

  return (
    <section
      role="status"
      aria-live="polite"
      aria-label="Update available"
      className="fixed right-3 bottom-8 z-50 w-[min(340px,calc(100vw-24px))] rounded-lg border bg-popover p-3.5 text-popover-foreground shadow-lg/10 duration-300 animate-in fade-in slide-in-from-bottom-2"
    >
      <div className="flex items-start gap-2.5">
        <ArrowUpCircle className="mt-0.5 size-4 shrink-0 text-primary" />
        <div className="min-w-0 flex-1">
          <h2 className="text-[13px] font-semibold">PDF Tutor {latest.version} is available</h2>
          <p className="text-xs text-muted-foreground">You have {check.current}.</p>
        </div>
        {!running && (
          <button
            aria-label="Close"
            onClick={dismissUpdate}
            className="-mt-1 -mr-1.5 grid size-6 place-items-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
          >
            <X className="size-3.5" />
          </button>
        )}
      </div>

      {running ? (
        <div className="mt-3 pl-6.5">
          <div className="mb-1.5 flex items-center justify-between text-xs">
            <span className="flex items-center gap-1.5"><Spinner className="size-3.5" />{status.step}…</span>
            {status.progress !== null && <span className="text-muted-foreground tabular-nums">{Math.round(status.progress * 100)}%</span>}
          </div>
          <Progress value={status.progress !== null ? status.progress * 100 : null} aria-label={status.step}>
            <ProgressTrack className="h-1"><ProgressIndicator /></ProgressTrack>
          </Progress>
          <p className="mt-1.5 text-xs text-muted-foreground">PDF Tutor restarts by itself when it's done.</p>
        </div>
      ) : (
        <>
          {notes.length > 0 && (
            <ul className="mt-2 list-disc space-y-0.5 pl-10 text-xs leading-relaxed text-muted-foreground">
              {notes.map((n) => <li key={n}>{n}</li>)}
              {more > 0 && <li>and {more} more</li>}
            </ul>
          )}
          {failed && <p className="mt-2 pl-6.5 text-xs text-destructive-foreground select-text">{status.message}</p>}
          {check.unavailable && <p className="mt-2 pl-6.5 text-xs text-muted-foreground">{check.unavailable}</p>}
          <div className="mt-3 flex items-center justify-between gap-2 pl-6.5">
            <button className="text-xs whitespace-nowrap text-primary hover:underline" onClick={() => openUrl(latest.url)}>Release notes</button>
            <div className="flex gap-1.5">
              <Button variant="ghost" size="sm" onClick={dismissUpdate}>Later</Button>
              {canInstall ? (
                <Button size="sm" onClick={startUpdate}>{failed ? "Try again" : "Update and restart"}</Button>
              ) : (
                <Button size="sm" onClick={() => openUrl(latest.url)}>Download</Button>
              )}
            </div>
          </div>
        </>
      )}
    </section>
  );
}
