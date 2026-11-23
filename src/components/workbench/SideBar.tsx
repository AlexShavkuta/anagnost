import { useApp } from "@/lib/store";
import { LibraryView } from "../views/LibraryView";
import { MapView } from "../views/MapView";
import { NotesView } from "../views/NotesView";
import { SearchView } from "../views/SearchView";

export function SideBar() {
  const view = useApp((s) => s.view);
  return (
    <aside aria-label="Side Bar" className="h-full bg-chrome">
      {view === "map" && <MapView />}
      {view === "search" && <SearchView />}
      {view === "notes" && <NotesView />}
      {view === "library" && <LibraryView />}
    </aside>
  );
}
