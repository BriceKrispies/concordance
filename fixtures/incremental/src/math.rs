/// Adds two integers.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

/// Clamps a percentage to the range `0..=100`.
pub fn clamp_percent(v: u32) -> u32 {
    v.min(100)
}
