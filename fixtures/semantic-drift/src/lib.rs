//! A mixer with a volume control.

/// The maximum volume the mixer is specified to allow.
pub const MAX_VOLUME: u8 = 100;

/// Holds the current output volume.
#[derive(Default)]
pub struct Mixer {
    volume: u8,
}

impl Mixer {
    /// Sets the output volume.
    pub fn set_volume(&mut self, v: u8) {
        self.volume = v.min(150);
    }

    /// Returns the current output volume.
    pub fn volume(&self) -> u8 {
        self.volume
    }
}
