# Mixer volume

## Overview

The mixer owns a single output volume that the audio path reads on every
block. Keeping the volume in a known range protects the downstream amplifier,
so the limit is a hard rule rather than a preference.

## Requirements

The volume must never exceed 100.

`Mixer::set_volume()` must clamp the requested volume to `MAX_VOLUME`.

`Mixer::volume()` returns the current volume.

## Example

```rust
use semantic_drift::Mixer;

let mut mixer = Mixer::default();
mixer.set_volume(42);
assert_eq!(mixer.volume(), 42);
```
