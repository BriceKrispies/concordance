# Expected: csharp

A C# project (xUnit tests) checked through the C# plugin.

| Spec statement | Expected finding |
|---|---|
| `ResetAll()` MUST restore every channel | violated: `ResetAll` does not exist in `src/` (exact md + no .cs target) |
| documented `public static int ParseLevel(string text);` | violated: actual signature has `int radix = 10` (`src/Mixer.cs`, line 25) |
| `ParseLevel()` MUST NOT throw | violated: `throw new FormatException(...)` in `src/Mixer.cs`, line 29 |
| **VOL-001** `Mixer.SetVolume()` SHALL reject a level above 100 | unverified until a run is ingested; verified when the TRX report passes `Volume.MixerTests.Rejects_level_above_100`; violated when it fails |
| `Mixer.Volume` | resolves to the property (no dangling reference) |

Exercised by `tests/tests/csharp.rs`.
