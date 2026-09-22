// Canonical scoping oracle: runs the published Loretta nuget package over a
// source read from stdin and prints a canonical, index-independent description
// of the scope tree (kinds, nesting, declared variables and goto labels).
//
// Usage: dotnet run --project tools/loretta-oracle [preset] < source.lua
using System.Collections.Immutable;
using System.Text.Json;
using Loretta.CodeAnalysis;
using Loretta.CodeAnalysis.Lua;

var preset = args.Length > 0 ? args[0] : "Luau";
var code = Console.In.ReadToEnd();

var options = preset switch
{
    "Luau" => LuaSyntaxOptions.Luau,
    "All" => LuaSyntaxOptions.All,
    "Lua51" => LuaSyntaxOptions.Lua51,
    "Lua52" => LuaSyntaxOptions.Lua52,
    "Lua53" => LuaSyntaxOptions.Lua53,
    "Lua54" => LuaSyntaxOptions.Lua54,
    "LuaJIT" => LuaSyntaxOptions.LuaJIT21,
    _ => throw new ArgumentException($"unknown preset `{preset}`"),
};

var tree = SyntaxFactory.ParseSyntaxTree(code, new LuaParseOptions(options));
var script = new Script(ImmutableArray.Create(tree));

var diagnostics = tree.GetDiagnostics().Select(d => d.ToString()).ToArray();

static string Canonical(IScope scope)
{
    var variables = scope.DeclaredVariables
        .Select(v => $"{v.Name}:{v.Kind}")
        .OrderBy(v => v, StringComparer.Ordinal)
        .ToArray();
    var labels = scope.GotoLabels
        .Select(l => l.Name)
        .OrderBy(l => l, StringComparer.Ordinal)
        .ToArray();
    var children = scope.ContainedScopes
        .Select(Canonical)
        .OrderBy(c => c, StringComparer.Ordinal)
        .ToArray();

    var variableText = string.Join(",", variables);
    var labelText = string.Join(",", labels);
    var childText = string.Join(",", children);
    return $"{scope.Kind}(vars=[{variableText}];labels=[{labelText}];children=[{childText}])";
}

Console.WriteLine(JsonSerializer.Serialize(new
{
    diagnostics,
    canonical = Canonical(script.RootScope),
}));
