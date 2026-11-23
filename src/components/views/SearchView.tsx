import { FileSearch, Search } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { InputGroup, InputGroupAddon, InputGroupInput } from "@/components/ui/input-group";
import { api, type SearchHit } from "@/lib/api";
import { useApp } from "@/lib/store";
import { EmptyState, ViewHeader } from "../workbench/common";

function highlight(text: string, q: string) {
  if (!q) return text;
  const parts = text.split(new RegExp(`(${q.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")})`, "gi"));
  return parts.map((p, i) => (i % 2 ? <mark key={i} className="rounded-[2px] bg-primary/15 text-inherit">{p}</mark> : p));
}

export function SearchView() {
  const doc = useApp((s) => s.doc);
  const missing = useApp((s) => s.textStatus.missing.length);
  const focusSeq = useApp((s) => s.searchFocusSeq);
  const [q, setQ] = useState("");
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, [focusSeq]);

  useEffect(() => {
    const query = q.trim();
    if (!doc || !query) return setHits(null);
    let live = true;
    const t = setTimeout(() => api.search(doc.id, query).then((h) => live && setHits(h)), 180);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [q, doc]);

  return (
    <div className="flex h-full flex-col">
      <ViewHeader title="Search" />
      <div className="px-3 pb-2">
        <InputGroup className="text-[13px] sm:text-[13px]">
          <InputGroupAddon>
            <Search />
          </InputGroupAddon>
          <InputGroupInput
            ref={input}
            size="sm"
            type="search"
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder={doc ? "Search the document" : "Open a PDF to search it"}
            disabled={!doc}
            spellCheck={false}
          />
          {hits && hits.length > 0 && (
            <InputGroupAddon align="inline-end">
              <Badge variant="secondary" className="tabular-nums">{hits.length >= 30 ? "30+" : hits.length}</Badge>
            </InputGroupAddon>
          )}
        </InputGroup>
      </div>
      {hits && hits.length === 0 && (
        <EmptyState icon={<FileSearch />} title="No results">
          {missing ? `${missing} pages have no text yet. Recognize them from the Map view.` : `Nothing on any page matches “${q.trim()}”.`}
        </EmptyState>
      )}
      <ul className="min-h-0 flex-1 overflow-auto pb-3">
        {hits?.map((h) => (
          <li key={h.page}>
            <button onClick={() => useApp.getState().goToPage(h.page)} className="block w-full px-4 py-1 text-left hover:bg-accent/70">
              <div className="text-[11px] text-muted-foreground tabular-nums">Page {h.page}</div>
              <div className="line-clamp-2 text-xs leading-snug">{highlight(h.snippet, q.trim())}</div>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
