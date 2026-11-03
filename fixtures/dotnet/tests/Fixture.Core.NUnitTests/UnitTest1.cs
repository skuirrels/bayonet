using Fixture.Core;

namespace Fixture.Core.NUnitTests;

public class GreeterTests
{
    [Test]
    public void Greets_by_name() => Assert.That(new Greeter().Greet("Ada"), Is.EqualTo("Hello, Ada!"));

    [TestCase("")]
    [TestCase(" ")]
    public void Rejects_blank_names(string name) => Assert.Throws(Is.InstanceOf<ArgumentException>(), () => new Greeter().Greet(name));
}
