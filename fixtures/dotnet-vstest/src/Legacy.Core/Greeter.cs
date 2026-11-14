namespace Legacy.Core;

public sealed class Greeter
{
    public string Greet(string name)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(name);
        return $"Hello, {name}!";
    }

    // Never called: a coverage run shows it uncovered.
    public string Farewell(string name) => $"Goodbye, {name}.";
}
