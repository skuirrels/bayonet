using Fixture.Core;

namespace Fixture.Core.MSTests;

[TestClass]
public sealed class GreeterTests
{
    [TestMethod]
    public void Greets_by_name() => Assert.AreEqual("Hello, Ada!", new Greeter().Greet("Ada"));
}
