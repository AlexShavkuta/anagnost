// Follow the system light/dark setting by toggling .dark on <html>.
import { useSyncExternalStore } from "react";

const query = window.matchMedia("(prefers-color-scheme: dark)");
const apply = () => document.documentElement.classList.toggle("dark", query.matches);
apply();
query.addEventListener("change", apply);

export function useDark(): boolean {
  return useSyncExternalStore(
    (cb) => {
      query.addEventListener("change", cb);
      return () => query.removeEventListener("change", cb);
    },
    () => query.matches,
  );
}
