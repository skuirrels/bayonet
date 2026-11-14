using Legacy.Core;

namespace Legacy.Core.MSTests;

[TestClass]
public sealed class GreeterTests
{
    [TestMethod]
    public void Greets_by_name() => Assert.AreEqual("Hello, Ada!", new Greeter().Greet("Ada"));

    [TestMethod]
    [DataRow("")]
    [DataRow(" ")]
    public void Rejects_blank_names(string name) => Assert.ThrowsExactly<ArgumentException>(() => new Greeter().Greet(name));
}
