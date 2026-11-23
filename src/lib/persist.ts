// Per-device UI preferences (panel visibility, zoom). Failing storage just means defaults.

export function load<T>(key: string, fallback: T): T {
  try {
    const v = localStorage.getItem(`pdf-tutor.${key}`);
    return v === null ? fallback : (JSON.parse(v) as T);
  } catch {
    return fallback;
  }
}

export function save(key: string, value: unknown) {
  try {
    localStorage.setItem(`pdf-tutor.${key}`, JSON.stringify(value));
  } catch {
    /* storage unavailable */
  }
}
