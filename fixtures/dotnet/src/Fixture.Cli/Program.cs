using Fixture.Core;

IGreeter greeter = new Greeter();
Console.WriteLine(greeter.Greet(args.Length > 0 ? args[0] : "world"));
