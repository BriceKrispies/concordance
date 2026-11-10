export const MAX_VOLUME = 100;

/** A mixer channel with a bounded volume. */
export class Mixer {
  private level = 0;

  /** Sets the volume; levels above MAX_VOLUME are rejected. */
  setVolume(v: number): void {
    if (v > MAX_VOLUME) {
      throw new RangeError(`volume ${v} exceeds ${MAX_VOLUME}`);
    }
    this.level = v;
  }

  get volume(): number {
    return this.level;
  }
}

/** Parses a textual level such as "42". */
export function parseLevel(text: string, radix = 10): number {
  const n = parseInt(text, radix);
  if (Number.isNaN(n)) {
    throw new TypeError(`not a level: ${text}`);
  }
  return n;
}
