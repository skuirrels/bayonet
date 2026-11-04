using Fixture.Core;

namespace Fixture.Core.XunitTests;

public class GreeterTests
{
    [Fact]
    public void Greets_by_name() => Assert.Equal("Hello, Ada!", new Greeter().Greet("Ada"));

    [Theory]
    [InlineData("")]
    [InlineData(" ")]
    public void Rejects_blank_names(string name) => Assert.ThrowsAny<ArgumentException>(() => new Greeter().Greet(name));
}
