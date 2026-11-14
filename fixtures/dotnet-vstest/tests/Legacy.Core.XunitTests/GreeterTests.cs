using Legacy.Core;

namespace Legacy.Core.XunitTests;

public class GreeterTests
{
    [Fact]
    public void Greets_by_name() => Assert.Equal("Hello, Ada!", new Greeter().Greet("Ada"));

    [Theory]
    [InlineData("")]
    [InlineData(" ")]
    public void Rejects_blank_names(string name) => Assert.ThrowsAny<ArgumentException>(() => new Greeter().Greet(name));

    [Fact]
    public void Fails_on_request()
    {
        Console.WriteLine("checking LEGACY_FAIL");
        Assert.True(Environment.GetEnvironmentVariable("LEGACY_FAIL") != "1", "LEGACY_FAIL is set");
    }

    [Fact(Skip = "not ready")]
    public void Is_skipped() { }
}
