# Volume mixer

This specification is normative.

## Clamping

**VOL-001** `Volume.clampLevel` SHALL never return a level above `Volume.maxLevel`.

**VOL-002** `clampLevel(n)` SHALL be idempotent: clamping an already clamped level changes nothing.

`resetAll()` MUST restore every channel to level 0.

```lean
def clampLevel (level : Nat) (floor : Nat) : Nat
```
