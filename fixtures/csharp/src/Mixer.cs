using System;

namespace Volume;

/// <summary>A mixer channel with a bounded volume.</summary>
public class Mixer
{
    public const int MaxVolume = 100;

    private int _level;

    /// <summary>Sets the volume; levels above MaxVolume are rejected.</summary>
    public void SetVolume(int v)
    {
        if (v > MaxVolume)
        {
            throw new ArgumentOutOfRangeException(nameof(v));
        }
        _level = v;
    }

    public int Volume => _level;

    /// <summary>Parses a textual level such as "42".</summary>
    public static int ParseLevel(string text, int radix = 10)
    {
        if (!int.TryParse(text, out var n))
        {
            throw new FormatException($"not a level: {text}");
        }
        return n;
    }
}
