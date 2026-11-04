var builder = DistributedApplication.CreateBuilder(args);

builder.AddProject<Projects.Fixture_Api>("api")
    .WithHttpHealthCheck("/weatherforecast");

builder.Build().Run();
