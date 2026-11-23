import { Check, CircleAlert, CircleCheck, CircleDashed, Copy, ExternalLink } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { message } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Button } from "@/components/ui/button";
import { Dialog, DialogDescription, DialogFooter, DialogHeader, DialogPanel, DialogPopup, DialogTitle } from "@/components/ui/dialog";
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { SegmentedControl, SegmentedControlItem } from "@/components/ui/segmented-control";
import { api, cursorInstallLink, cursorMcpJson, type OcrInfo, type Provider } from "@/lib/api";
import { useApp } from "@/lib/store";
import { IconButton } from "./common";

function Status({ state, children }: { state: "ok" | "bad" | "todo"; children: ReactNode }) {
  const Icon = state === "ok" ? CircleCheck : state === "bad" ? CircleAlert : CircleDashed;
  const color = state === "ok" ? "text-success" : state === "bad" ? "text-destructive-foreground" : "text-muted-foreground";
  return (
    <li className="flex gap-2 py-0.5 text-xs leading-relaxed">
      <Icon className={`mt-0.5 size-3.5 shrink-0 ${color}`} />
      <span>{children}</span>
    </li>
  );
}

function CopyBlock({ id, text }: { id?: string; text: string }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => setCopied(false), [text]);
  return (
    <div className="flex w-full items-start gap-1">
      <code id={id} className="flex-1 rounded-lg border bg-muted/50 px-2.5 py-1.5 font-mono text-[11.5px] leading-relaxed break-all whitespace-pre-wrap select-all">
        {text}
      </code>
      <IconButton label={copied ? "Copied" : "Copy"} onClick={() => navigator.clipboard.writeText(text).then(() => setCopied(true))}>
        {copied ? <Check /> : <Copy />}
      </IconButton>
    </div>
  );
}

/** A two-option segmented control (coss), with radio semantics. */
function Choice<T extends string>({ label, value, onChange, options, id }: {
  label: string; value: T; onChange(v: T): void; options: [T, string][]; id?: string;
}) {
  return (
    <SegmentedControl id={id} role="radiogroup" aria-label={label} size="sm" className="mb-3 w-full">
      {options.map(([v, text]) => (
        <SegmentedControlItem
          key={v}
          role="radio"
          aria-checked={value === v}
          data-checked={value === v ? "" : undefined}
          onClick={() => onChange(v)}
          className="flex-1 text-xs sm:text-xs"
        >
          {text}
        </SegmentedControlItem>
      ))}
    </SegmentedControl>
  );
}

const code = "rounded-sm bg-muted px-1 font-mono text-[11px]";
const input = "text-[13px] sm:text-[13px]";
const field = "gap-1.5 [&+&]:mt-3";

export function SettingsDialog() {
  const open = useApp((s) => s.settingsOpen);
  const focus = useApp((s) => s.settingsFocus);
  const settings = useApp((s) => s.settings);
  const ocrServer = useApp((s) => s.ocrServer);
  const { closeSettings, loadSettings } = useApp.getState();
  const [provider, setProvider] = useState<Provider>("anthropic");
  const [key, setKey] = useState("");
  const [model, setModel] = useState("");
  const [cursorKey, setCursorKey] = useState("");
  const [cursorModel, setCursorModel] = useState("");
  const [cursorPath, setCursorPath] = useState("");
  const [llama, setLlama] = useState("");
  const [ocr, setOcr] = useState<OcrInfo | null>(null);
  const [agent, setAgent] = useState<"claude" | "cursor">("claude");

  useEffect(() => {
    if (!open || !settings) return;
    setProvider(settings.provider);
    setKey("");
    setModel(settings.model);
    setCursorKey("");
    setCursorModel(settings.cursor_model);
    setCursorPath(settings.cursor_agent_path);
    setLlama(settings.llama_server_path);
    if (focus === "mcp-cursor") setAgent("cursor");
    api.ocrInfo().then(setOcr);
    if (focus) {
      if (focus === "cursor-key" || focus === "cursor-path") setProvider("cursor");
      if (focus === "api-key") setProvider("anthropic");
      setTimeout(() => {
        const el = document.getElementById(focus);
        el?.scrollIntoView({ block: "center" });
        if (el instanceof HTMLInputElement) el.focus();
      }, 60);
    }
  }, [open, settings?.mcp_token]);

  if (!settings) return null;
  const status = ocrServer.state !== "stopped" ? ocrServer : ocr?.status;

  const save = async () => {
    try {
      await api.saveSettings({
        provider,
        apiKey: key.trim() || undefined,
        model,
        cursorApiKey: cursorKey.trim() || undefined,
        cursorModel,
        cursorAgentPath: cursorPath,
        llamaServerPath: llama,
      });
      await loadSettings();
      useApp.setState({ chatError: "" });
      closeSettings();
    } catch (e) {
      await message(`Could not save settings.\n\n${e}`, { title: "Settings", kind: "error" });
    }
  };

  return (
    <Dialog open={open} onOpenChange={(o) => !o && closeSettings()}>
      <DialogPopup className="max-w-[600px]" initialFocus={false}>
        <DialogHeader className="px-5 pt-4 pb-2">
          <DialogTitle className="text-[15px] font-semibold">Settings</DialogTitle>
          <DialogDescription className="sr-only">Model provider, agent connections and text recognition</DialogDescription>
        </DialogHeader>
        <DialogPanel className="px-5 py-0">
          <form
            id="settings-form"
            onSubmit={(e) => {
              e.preventDefault();
              save();
            }}
          >
            <section className="pt-1 pb-4">
              <h3 className="mb-0.5 font-semibold">Tutor and map</h3>
              <p className="mb-2 text-xs text-muted-foreground">Which account answers in the tutor and builds the map. Keys are stored in the macOS Keychain.</p>
              <Choice<Provider> id="provider" label="Provider" value={provider} onChange={setProvider} options={[["anthropic", "Anthropic API key"], ["cursor", "Cursor API key"]]} />
              {provider === "anthropic" ? (
                <>
                  <Field className={field}>
                    <FieldLabel className="text-xs sm:text-xs">API key</FieldLabel>
                    <Input id="api-key" type="password" autoComplete="off" placeholder="sk-ant-…" value={key} onChange={(e) => setKey(e.target.value)} className={input} />
                    <FieldDescription>From console.anthropic.com.{settings.has_api_key && " A key is saved; leave this empty to keep it."}</FieldDescription>
                  </Field>
                  <Field className={field}>
                    <FieldLabel className="text-xs sm:text-xs">Model</FieldLabel>
                    <Input id="model" spellCheck={false} value={model} onChange={(e) => setModel(e.target.value)} className={input} />
                  </Field>
                </>
              ) : (
                <>
                  <Field className={field}>
                    <FieldLabel className="text-xs sm:text-xs">Cursor API key</FieldLabel>
                    <Input id="cursor-key" type="password" autoComplete="off" placeholder="key_…" value={cursorKey} onChange={(e) => setCursorKey(e.target.value)} className={input} />
                    <FieldDescription>
                      Create one in the Cursor dashboard under Integrations. Requests run on your Cursor plan.
                      {settings.has_cursor_key && " A key is saved; leave this empty to keep it."}
                    </FieldDescription>
                  </Field>
                  <Field className={field}>
                    <FieldLabel className="text-xs sm:text-xs">Model</FieldLabel>
                    <Input id="cursor-model" spellCheck={false} placeholder="Cursor's default" value={cursorModel} onChange={(e) => setCursorModel(e.target.value)} className={input} />
                    <FieldDescription>
                      Any model your plan offers, e.g. <code className={code}>sonnet-4.5</code> or <code className={code}>gpt-5</code>. Leave empty for Cursor's default.
                    </FieldDescription>
                  </Field>
                  <ul className="mt-3 mb-3">
                    {settings.cursor_agent ? (
                      <Status state="ok">cursor-agent found at <code className={code}>{settings.cursor_agent}</code></Status>
                    ) : (
                      <Status state="bad">cursor-agent not found. Install Cursor's CLI with <code className={code}>curl https://cursor.com/install -fsS | bash</code>, or set its path below.</Status>
                    )}
                    <Status state="todo">It runs read-only: ask mode, in an empty temporary folder, with shell commands and file writes denied.</Status>
                  </ul>
                  <Field className={field}>
                    <FieldLabel className="text-xs sm:text-xs">cursor-agent path</FieldLabel>
                    <Input id="cursor-path" spellCheck={false} placeholder="~/.local/bin/cursor-agent" value={cursorPath} onChange={(e) => setCursorPath(e.target.value)} className={input} />
                    <FieldDescription>Leave empty to find it automatically.</FieldDescription>
                  </Field>
                </>
              )}
            </section>

            <section className="border-t py-4">
              <h3 className="mb-0.5 font-semibold">Connect an agent</h3>
              <p className="mb-2 text-xs text-muted-foreground">
                PDF Tutor runs an MCP server while it's open. Connect Claude Code or Cursor, then ask it about the PDF you have open: it can read the
                document, see your page and selection, move the viewer, and write to the map and notes.
              </p>
              <Choice<"claude" | "cursor"> label="Agent" value={agent} onChange={setAgent} options={[["claude", "Claude Code"], ["cursor", "Cursor"]]} />
              {agent === "claude" ? (
                <>
                  <p className="mb-1.5 text-xs text-muted-foreground">Run this once in a terminal:</p>
                  <CopyBlock id="mcp-cmd" text={`claude mcp add --transport http pdf-tutor ${settings.mcp_url} --header "Authorization: Bearer ${settings.mcp_token}"`} />
                </>
              ) : (
                <>
                  <div id="mcp-cursor" className="mb-2 flex items-center gap-2">
                    <Button size="sm" variant="outline" onClick={() => openUrl(cursorInstallLink(settings)).catch((e) => message(String(e), { kind: "error" }))}>
                      <ExternalLink />
                      Add to Cursor
                    </Button>
                    <span className="text-xs text-muted-foreground">Opens Cursor and asks you to confirm.</span>
                  </div>
                  <p className="mb-1.5 text-xs text-muted-foreground">
                    Or add this to <code className={code}>~/.cursor/mcp.json</code> (it's also what <code className={code}>cursor-agent</code> reads):
                  </p>
                  <CopyBlock text={cursorMcpJson(settings)} />
                </>
              )}
              {settings.mcp_error && <p className="mt-2 text-xs text-destructive-foreground">The MCP server isn't running: {settings.mcp_error}</p>}
            </section>

            <section className="border-t py-4">
              <h3 className="mb-0.5 font-semibold">Text recognition</h3>
              <p className="mb-2 text-xs text-muted-foreground">
                Scanned pages have no text, so the tutor can't read them. PDF Tutor recognizes them on this Mac with{" "}
                <a className="text-primary hover:underline" href="https://huggingface.co/baidu/Qianfan-OCR">Qianfan-OCR</a> through llama.cpp. Nothing leaves your Mac.
              </p>
              {ocr && (
                <ul className="mb-3">
                  {ocr.binary ? (
                    <Status state="ok">llama-server found at <code className={code}>{ocr.binary}</code></Status>
                  ) : (
                    <Status state="bad">llama-server not found. Install llama.cpp with <code className={code}>brew install llama.cpp</code>, or set its path below.</Status>
                  )}
                  {ocr.downloaded ? (
                    <Status state="ok">Model downloaded (<code className={code}>{ocr.model}</code>)</Status>
                  ) : (
                    <Status state="todo">Model not downloaded yet. About 4.8 GB is downloaded from Hugging Face the first time you recognize text.</Status>
                  )}
                  {status?.state === "ready" ? (
                    <Status state="ok">Model loaded and ready</Status>
                  ) : status?.state === "error" ? (
                    <Status state="bad">Last start failed: {status.message}</Status>
                  ) : status?.state === "starting" ? (
                    <Status state="todo">{status.detail}{status.progress !== null ? ` ${Math.round(status.progress * 100)}%` : "…"}</Status>
                  ) : (
                    <Status state="todo">Model not loaded. It starts when you recognize text and unloads after 5 idle minutes.</Status>
                  )}
                </ul>
              )}
              <Field className={field}>
                <FieldLabel className="text-xs sm:text-xs">llama-server path</FieldLabel>
                <Input id="llama-path" spellCheck={false} placeholder="/opt/homebrew/bin/llama-server" value={llama} onChange={(e) => setLlama(e.target.value)} className={input} />
                <FieldDescription>Leave empty to find it automatically.</FieldDescription>
              </Field>
            </section>
          </form>
        </DialogPanel>
        <DialogFooter className="px-5 py-3">
          <Button variant="outline" size="sm" onClick={closeSettings}>Cancel</Button>
          <Button size="sm" type="submit" form="settings-form">Save</Button>
        </DialogFooter>
      </DialogPopup>
    </Dialog>
  );
}
