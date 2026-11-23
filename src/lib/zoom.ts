import { create } from "zustand";
import { load, save } from "./persist";

interface Zoom {
  scale: number;
  /** Fit the page width to the viewer; on by default and after ⌘0. */
  fit: boolean;
  setScale(scale: number, fit?: boolean): void;
  zoomBy(factor: number): void;
  fitWidth(): void;
}

const clamp = (s: number) => Math.min(4, Math.max(0.3, s));

export const useZoom = create<Zoom>()((set, get) => ({
  scale: 1.25,
  fit: load("fit", true),
  setScale(scale, fit = get().fit) {
    set({ scale: clamp(scale), fit });
  },
  zoomBy(factor) {
    set({ scale: clamp(Math.round(get().scale * factor * 20) / 20), fit: false });
    save("fit", false);
  },
  fitWidth() {
    set({ fit: true });
    save("fit", true);
  },
}));
