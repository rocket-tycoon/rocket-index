# fsharp-tools Scripts

This directory contains scripts for extending fsharp-tools functionality.

## extract-types.fsx

An F# script that extracts type information from F# projects using FSharp.Compiler.Service (FCS). This is part of [RFC-001: Hybrid Type Architecture](../design/RFC-001-hybrid-type-architecture.md).

### Purpose

The script runs at build time (not runtime) to extract type information that the Rust-based fsharp-tools can use for type-aware symbol resolution. This enables features like:

- Resolving member access expressions (e.g., `user.Name` → navigating to the `Name` property on `User`)
- Showing type signatures in hover information
- More accurate dependency graphs in spider

### Usage

```bash
# Basic usage
dotnet fsi scripts/extract-types.fsx path/to/MyProject.fsproj

# With custom output directory
dotnet fsi scripts/extract-types.fsx path/to/MyProject.fsproj --output .my-cache/

# With verbose logging
dotnet fsi scripts/extract-types.fsx path/to/MyProject.fsproj --verbose
```

### Options

| Option | Description |
|--------|-------------|
| `--output, -o <path>` | Output directory for type cache (default: `.fsharp-types/` in project directory) |
| `--verbose, -v` | Enable verbose output showing all extracted symbols |
| `--help, -h` | Show help message |

### Output

The script creates a `cache.json` file in the output directory with the following structure:

```json
{
  "version": 1,
  "extracted_at": "2024-12-02T10:30:00Z",
  "project": "MyProject",
  "symbols": [
    {
      "name": "processUser",
      "qualified": "MyApp.UserService.processUser",
      "type": "User -> Async<Result<Response, Error>>",
      "file": "src/UserService.fs",
      "line": 15,
      "parameters": [
        { "name": "user", "type": "User" }
      ]
    }
  ],
  "members": [
    {
      "type": "User",
      "member": "Name",
      "member_type": "string",
      "kind": "property"
    }
  ]
}
```

### Integration

#### With MSBuild (post-build hook)

Add to your `.fsproj`:

```xml
<Target Name="ExtractFSharpTypes" AfterTargets="Build">
  <Exec Command="dotnet fsi $(MSBuildThisFileDirectory)../scripts/extract-types.fsx $(MSBuildProjectFullPath)" 
        ContinueOnError="true" />
</Target>
```

#### With fsharp-index CLI

```bash
# Run indexing with type extraction
fsharp-index build --extract-types

# Or run extraction separately
fsharp-index extract-types MyProject.fsproj
```

### Requirements

- .NET 6.0 or later
- FSharp.Compiler.Service (automatically downloaded via `#r "nuget:..."`)

### Performance

- First run: ~5-10 seconds (downloads NuGet packages)
- Subsequent runs: ~2-5 seconds for typical projects
- Memory: Proportional to project size, released after script completes

### Troubleshooting

**"Project file not found"**
- Ensure the path to `.fsproj` is correct
- Use absolute path if relative path isn't working

**"Type checking aborted"**
- The project may have compilation errors
- Run `dotnet build` first to see any errors

**"No implementation file found"**
- Script files (`.fsx`) don't have implementation files
- Use with `.fsproj` files that compile to `.dll`

**Slow extraction**
- First run downloads FSharp.Compiler.Service (~50MB)
- Use `--verbose` to see progress