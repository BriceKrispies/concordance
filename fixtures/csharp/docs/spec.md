# Volume mixer

This specification is normative.

## Mixer

**VOL-001** `Mixer.SetVolume()` SHALL reject a level above 100 with an `ArgumentOutOfRangeException`.

`Mixer.Volume` returns the current level.

`ResetAll()` MUST restore every channel to level 0.

## Parsing

`ParseLevel()` MUST NOT throw.

```csharp
public static int ParseLevel(string text);
```
