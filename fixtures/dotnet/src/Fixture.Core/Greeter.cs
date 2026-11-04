namespace Fixture.Core;

public interface IGreeter
{
    string Greet(string name);
}

public sealed class Greeter : IGreeter
{
    public string Greet(string name)
    {
#if NET10_0_OR_GREATER
        ArgumentException.ThrowIfNullOrWhiteSpace(name);
#else
        if (string.IsNullOrWhiteSpace(name)) throw new ArgumentException("Name is required.", nameof(name));
#endif
        return $"Hello, {name}!";
    }
}
