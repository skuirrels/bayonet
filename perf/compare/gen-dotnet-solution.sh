#!/bin/sh
# Generates the NFR-1 / NFR-2 benchmark solution: <count> chained class libraries (the last
# one an executable), three source files each, in a .slnx. Each module calls the one before
# it, so project evaluation and cross-project analysis both have real work to do.
#   perf/compare/gen-dotnet-solution.sh <dir> [count=100]
set -eu

dir=${1:?usage: gen-dotnet-solution.sh <dir> [count]}
count=${2:-100}
name=$(basename "$dir")
mkdir -p "$dir"
cd "$dir"

cat >Directory.Build.props <<'EOF'
<Project>
  <PropertyGroup>
    <TargetFramework>net10.0</TargetFramework>
    <ImplicitUsings>enable</ImplicitUsings>
    <Nullable>enable</Nullable>
  </PropertyGroup>
</Project>
EOF

n=1
while [ "$n" -le "$count" ]; do
  id=$(printf %03d "$n")
  prev=$(printf %03d $((n - 1)))
  mkdir -p "Module$id"
  refs=""
  use="x"
  if [ "$n" -gt 1 ]; then
    refs="<ItemGroup><ProjectReference Include=\"../Module$prev/Module$prev.csproj\" /></ItemGroup>"
    use="new Module$prev.Service1().Run(x)"
  fi
  out=""
  if [ "$n" -eq "$count" ]; then out="<OutputType>Exe</OutputType>"; fi
  echo "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup>$out</PropertyGroup>$refs</Project>" >"Module$id/Module$id.csproj"
  for k in 1 2 3; do
    cat >"Module$id/Service$k.cs" <<EOF
namespace Module$id;

public sealed class Service$k
{
    private readonly Dictionary<string, int> _counts = new();

    public int Run(int x)
    {
        var total = $use;
        foreach (var key in _counts.Keys.Where(k => k.Length > x % 7))
            total += _counts[key];
        return total + $k;
    }

    public void Add(string key) => _counts[key] = _counts.GetValueOrDefault(key) + 1;
}
EOF
  done
  n=$((n + 1))
done
echo "Console.WriteLine(new Module$(printf %03d "$count").Service1().Run(3));" >"Module$(printf %03d "$count")/Program.cs"

dotnet new sln -n "$name" --format slnx >/dev/null
dotnet sln "$name.slnx" add Module*/Module*.csproj >/dev/null
dotnet build -nologo -v:q
