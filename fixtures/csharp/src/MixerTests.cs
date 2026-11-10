using System;
using Xunit;

namespace Volume;

public class MixerTests
{
    // Verifies: VOL-001
    [Fact]
    public void Rejects_level_above_100()
    {
        Assert.Throws<ArgumentOutOfRangeException>(() => new Mixer().SetVolume(101));
    }

    [Fact]
    public void Keeps_an_accepted_level()
    {
        var m = new Mixer();
        m.SetVolume(40);
        Assert.Equal(40, m.Volume);
    }
}
