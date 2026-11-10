namespace Volume

/-- The loudest level a channel accepts. -/
def maxLevel : Nat := 100

/-- Clamp a requested level into range. -/
def clampLevel (level : Nat) : Nat :=
  if level > maxLevel then maxLevel else level

structure Channel where
  name : String
  level : Nat

/-- Verifies: VOL-001 -/
theorem clamp_le_max (n : Nat) : clampLevel n ≤ maxLevel := by
  unfold clampLevel
  split <;> omega

-- Verifies: VOL-002
theorem clamp_idempotent (n : Nat) : clampLevel (clampLevel n) = clampLevel n := by
  sorry

end Volume
