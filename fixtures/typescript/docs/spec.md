# Volume mixer

This specification is normative.

## Mixer

**VOL-001** `Mixer.setVolume()` SHALL reject a level above 100 with a `RangeError`.

`Mixer.volume` returns the current level.

`resetAll()` MUST restore every channel to level 0.

## Parsing

`parseLevel()` MUST NOT throw.

```ts
export function parseLevel(text: string): number;
```
