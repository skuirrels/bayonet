namespace Fixture.Core;

/// <summary>Code for the refactoring tests (CI-6): each refactoring starts somewhere in here.</summary>
public sealed class Pricing
{
    public decimal Total(decimal price, int quantity)
    {
        var subtotal = price * quantity;
        var discount = subtotal > 100 ? subtotal * 0.1m : 0m;
        return subtotal - discount;
    }

    public decimal Tax(decimal amount) => amount * 0.2m;
}

public sealed class Receipt
{
    public string Line(string item, decimal amount) => $"{item}: {amount}";
}
