#!/usr/bin/env dotnet fsi

//
// extract-types.fsx - Extract type information from F# projects using FCS
//
// Usage: dotnet fsi extract-types.fsx <path-to-fsproj> [--output <path>] [--verbose]
//
// This script is part of RFC-001: Hybrid Type Architecture for fsharp-tools.
// It extracts type information at build time and writes it directly to SQLite
// database that the Rust runtime can read for type-aware symbol resolution.
//
// The SQLite schema matches the Rust SqliteIndex implementation.
//

#r "nuget: FSharp.Compiler.Service, 43.8.400"
#r "nuget: Microsoft.Data.Sqlite, 8.0.0"

open System
open System.IO
open Microsoft.Data.Sqlite
open FSharp.Compiler.CodeAnalysis
open FSharp.Compiler.Symbols
open FSharp.Compiler.Text

// ============================================================================
// Schema Constants (must match Rust db.rs)
// ============================================================================

let SCHEMA_VERSION = 1

let SCHEMA_SQL =
    """
CREATE TABLE IF NOT EXISTS metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS symbols (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    qualified TEXT NOT NULL,
    kind TEXT NOT NULL,
    file TEXT NOT NULL,
    line INTEGER NOT NULL,
    column INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    end_column INTEGER NOT NULL,
    visibility TEXT NOT NULL DEFAULT 'public',
    type_signature TEXT,
    source TEXT NOT NULL DEFAULT 'syntactic'
);

CREATE INDEX IF NOT EXISTS idx_symbols_qualified ON symbols(qualified);
CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file);

CREATE TABLE IF NOT EXISTS members (
    id INTEGER PRIMARY KEY,
    type_name TEXT NOT NULL,
    member_name TEXT NOT NULL,
    member_type TEXT NOT NULL,
    kind TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_members_type ON members(type_name);
CREATE INDEX IF NOT EXISTS idx_members_name ON members(type_name, member_name);

CREATE TABLE IF NOT EXISTS refs (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    file TEXT NOT NULL,
    line INTEGER NOT NULL,
    column INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_refs_name ON refs(name);
CREATE INDEX IF NOT EXISTS idx_refs_file ON refs(file);

CREATE TABLE IF NOT EXISTS opens (
    id INTEGER PRIMARY KEY,
    file TEXT NOT NULL,
    module TEXT NOT NULL,
    line INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_opens_file ON opens(file);
"""

// ============================================================================
// Types
// ============================================================================

type MemberKind =
    | Property
    | Method
    | Field
    | Event

type TypeMember =
    { TypeName: string
      Member: string
      MemberType: string
      Kind: MemberKind }

type TypedSymbol =
    { Name: string
      Qualified: string
      Type: string
      File: string
      Line: int
      Column: int
      EndLine: int
      EndColumn: int }

// ============================================================================
// Command Line Parsing
// ============================================================================

type Options =
    { ProjectPath: string
      OutputPath: string option
      Verbose: bool }

let parseArgs (args: string[]) =
    let mutable projectPath = None
    let mutable outputPath = None
    let mutable verbose = false
    let mutable i = 0

    while i < args.Length do
        match args.[i] with
        | "--output"
        | "-o" when i + 1 < args.Length ->
            outputPath <- Some args.[i + 1]
            i <- i + 2
        | "--verbose"
        | "-v" ->
            verbose <- true
            i <- i + 1
        | "--help"
        | "-h" ->
            printfn "Usage: dotnet fsi extract-types.fsx <path-to-fsproj> [--output <path>] [--verbose]"
            printfn ""
            printfn "Options:"
            printfn "  --output, -o <path>  Output directory for SQLite database (default: .fsharp-index/)"
            printfn "  --verbose, -v        Enable verbose output"
            printfn "  --help, -h           Show this help"
            Environment.Exit(0)
            i <- i + 1
        | path when path.EndsWith(".fsproj") || path.EndsWith(".fsx") ->
            projectPath <- Some path
            i <- i + 1
        | unknown ->
            eprintfn "Unknown argument: %s" unknown
            Environment.Exit(1)
            i <- i + 1

    match projectPath with
    | None ->
        eprintfn "Error: No project path specified"
        eprintfn "Usage: dotnet fsi extract-types.fsx <path-to-fsproj> [--output <path>] [--verbose]"
        Environment.Exit(1)
        failwith "unreachable"
    | Some p ->
        { ProjectPath = p
          OutputPath = outputPath
          Verbose = verbose }

// ============================================================================
// SQLite Operations
// ============================================================================

let initDatabase (conn: SqliteConnection) =
    use cmd = conn.CreateCommand()
    cmd.CommandText <- SCHEMA_SQL
    cmd.ExecuteNonQuery() |> ignore

    // Set schema version
    use metaCmd = conn.CreateCommand()
    metaCmd.CommandText <- "INSERT OR REPLACE INTO metadata (key, value) VALUES ('schema_version', @version)"
    metaCmd.Parameters.AddWithValue("@version", SCHEMA_VERSION.ToString()) |> ignore
    metaCmd.ExecuteNonQuery() |> ignore

let insertSymbol (conn: SqliteConnection) (sym: TypedSymbol) =
    use cmd = conn.CreateCommand()

    cmd.CommandText <-
        """
        INSERT INTO symbols (name, qualified, kind, file, line, column, end_line, end_column, visibility, type_signature, source)
        VALUES (@name, @qualified, @kind, @file, @line, @column, @end_line, @end_column, @visibility, @type_sig, 'semantic')
    """

    cmd.Parameters.AddWithValue("@name", sym.Name) |> ignore
    cmd.Parameters.AddWithValue("@qualified", sym.Qualified) |> ignore
    cmd.Parameters.AddWithValue("@kind", "Function") |> ignore // Default kind
    cmd.Parameters.AddWithValue("@file", sym.File) |> ignore
    cmd.Parameters.AddWithValue("@line", sym.Line) |> ignore
    cmd.Parameters.AddWithValue("@column", sym.Column) |> ignore
    cmd.Parameters.AddWithValue("@end_line", sym.EndLine) |> ignore
    cmd.Parameters.AddWithValue("@end_column", sym.EndColumn) |> ignore
    cmd.Parameters.AddWithValue("@visibility", "public") |> ignore
    cmd.Parameters.AddWithValue("@type_sig", sym.Type) |> ignore
    cmd.ExecuteNonQuery() |> ignore

let updateSymbolType (conn: SqliteConnection) (qualified: string) (typeSig: string) =
    use cmd = conn.CreateCommand()

    cmd.CommandText <-
        """
        UPDATE symbols
        SET type_signature = @type_sig, source = 'semantic'
        WHERE qualified = @qualified
    """

    cmd.Parameters.AddWithValue("@qualified", qualified) |> ignore
    cmd.Parameters.AddWithValue("@type_sig", typeSig) |> ignore
    cmd.ExecuteNonQuery()

let insertMember (conn: SqliteConnection) (m: TypeMember) =
    use cmd = conn.CreateCommand()

    cmd.CommandText <-
        """
        INSERT INTO members (type_name, member_name, member_type, kind)
        VALUES (@type_name, @member_name, @member_type, @kind)
    """

    cmd.Parameters.AddWithValue("@type_name", m.TypeName) |> ignore
    cmd.Parameters.AddWithValue("@member_name", m.Member) |> ignore
    cmd.Parameters.AddWithValue("@member_type", m.MemberType) |> ignore

    let kindStr =
        match m.Kind with
        | Property -> "property"
        | Method -> "method"
        | Field -> "field"
        | Event -> "event"

    cmd.Parameters.AddWithValue("@kind", kindStr) |> ignore
    cmd.ExecuteNonQuery() |> ignore

let setMetadata (conn: SqliteConnection) (key: string) (value: string) =
    use cmd = conn.CreateCommand()
    cmd.CommandText <- "INSERT OR REPLACE INTO metadata (key, value) VALUES (@key, @value)"
    cmd.Parameters.AddWithValue("@key", key) |> ignore
    cmd.Parameters.AddWithValue("@value", value) |> ignore
    cmd.ExecuteNonQuery() |> ignore

let clearSemanticData (conn: SqliteConnection) =
    // Clear members (they're all from semantic extraction)
    use clearMembers = conn.CreateCommand()
    clearMembers.CommandText <- "DELETE FROM members"
    clearMembers.ExecuteNonQuery() |> ignore

    // Clear semantic symbols (keep syntactic ones)
    use clearSymbols = conn.CreateCommand()
    clearSymbols.CommandText <- "DELETE FROM symbols WHERE source = 'semantic'"
    clearSymbols.ExecuteNonQuery() |> ignore

// ============================================================================
// Type Extraction
// ============================================================================

let log verbose msg =
    if verbose then
        printfn "[extract-types] %s" msg

let formatType (t: FSharpType) : string =
    try
        t.Format(FSharpDisplayContext.Empty)
    with _ ->
        "unknown"

let getQualifiedName (entity: FSharpEntity) =
    try
        entity.FullName
    with _ ->
        entity.DisplayName

let getQualifiedNameForMemberOrVal (mfv: FSharpMemberOrFunctionOrValue) =
    try
        mfv.FullName
    with _ ->
        try
            match mfv.DeclaringEntity with
            | Some e -> sprintf "%s.%s" (getQualifiedName e) mfv.DisplayName
            | None -> mfv.DisplayName
        with _ ->
            mfv.DisplayName

let extractSymbolsFromDeclarations
    (projectDir: string)
    (verbose: bool)
    (decls: FSharpImplementationFileDeclaration list)
    : TypedSymbol list * TypeMember list =

    let symbols = ResizeArray<TypedSymbol>()
    let members = ResizeArray<TypeMember>()

    let rec processDecl (decl: FSharpImplementationFileDeclaration) =
        match decl with
        | FSharpImplementationFileDeclaration.Entity(entity, subDecls) ->
            // Process sub-declarations first
            for sub in subDecls do
                processDecl sub

            // Extract type members for records, unions, classes
            if entity.IsFSharpRecord then
                for field in entity.FSharpFields do
                    members.Add(
                        { TypeName = entity.DisplayName
                          Member = field.DisplayName
                          MemberType = formatType field.FieldType
                          Kind = Field }
                    )

                    log
                        verbose
                        (sprintf
                            "  Record field: %s.%s : %s"
                            entity.DisplayName
                            field.DisplayName
                            (formatType field.FieldType))

            elif entity.IsFSharpUnion then
                for case in entity.UnionCases do
                    let caseType =
                        if case.Fields.Count = 0 then
                            entity.DisplayName
                        else
                            let fieldTypes =
                                case.Fields |> Seq.map (fun f -> formatType f.FieldType) |> String.concat " * "

                            sprintf "%s of %s" entity.DisplayName fieldTypes

                    members.Add(
                        { TypeName = entity.DisplayName
                          Member = case.DisplayName
                          MemberType = caseType
                          Kind = Method // Union cases are constructor-like
                        }
                    )

                    log verbose (sprintf "  Union case: %s.%s" entity.DisplayName case.DisplayName)

            elif entity.IsFSharpModule then
                // Module members are handled in MemberOrFunctionOrValue
                ()

            else
                // Class, interface, or other type - extract members
                for mfv in entity.MembersFunctionsAndValues do
                    if not mfv.IsCompilerGenerated then
                        let kind =
                            if mfv.IsProperty || mfv.IsPropertyGetterMethod || mfv.IsPropertySetterMethod then
                                Property
                            elif mfv.IsEvent then
                                Event
                            else
                                Method

                        members.Add(
                            { TypeName = entity.DisplayName
                              Member = mfv.DisplayName
                              MemberType = formatType mfv.FullType
                              Kind = kind }
                        )

                        log
                            verbose
                            (sprintf "  Member: %s.%s : %s" entity.DisplayName mfv.DisplayName (formatType mfv.FullType))

        | FSharpImplementationFileDeclaration.MemberOrFunctionOrValue(mfv, _, _) ->
            if not mfv.IsCompilerGenerated then
                let range = mfv.DeclarationLocation

                let file =
                    try
                        let fullPath = range.FileName

                        if Path.IsPathRooted(fullPath) then
                            Path.GetRelativePath(projectDir, fullPath)
                        else
                            fullPath
                    with _ ->
                        "unknown"

                symbols.Add(
                    { Name = mfv.DisplayName
                      Qualified = getQualifiedNameForMemberOrVal mfv
                      Type = formatType mfv.FullType
                      File = file
                      Line = range.StartLine
                      Column = range.StartColumn + 1 // 1-indexed
                      EndLine = range.EndLine
                      EndColumn = range.EndColumn + 1 }
                )

                log
                    verbose
                    (sprintf "Symbol: %s : %s @ %s:%d" mfv.DisplayName (formatType mfv.FullType) file range.StartLine)

        | FSharpImplementationFileDeclaration.InitAction _ -> () // Skip module initialization actions

    for decl in decls do
        processDecl decl

    (symbols |> Seq.toList, members |> Seq.toList)

let extractFromCheckResults
    (projectDir: string)
    (verbose: bool)
    (checkResults: FSharpCheckFileResults)
    : TypedSymbol list * TypeMember list =

    match checkResults.ImplementationFile with
    | Some implFile -> extractSymbolsFromDeclarations projectDir verbose implFile.Declarations
    | None ->
        log verbose "No implementation file found"
        ([], [])

// ============================================================================
// Project Loading
// ============================================================================

let loadProject (projectPath: string) (verbose: bool) =
    async {
        log verbose (sprintf "Loading project: %s" projectPath)

        let checker = FSharpChecker.Create(keepAssemblyContents = true)
        let projectDir = Path.GetDirectoryName(Path.GetFullPath(projectPath))

        // Get project options from MSBuild
        log verbose "Cracking project..."
        let! projOptions, diagnostics = checker.GetProjectOptionsFromProjectFile(projectPath)

        if verbose then
            for diag in diagnostics do
                printfn "[project] %s" (diag.ToString())

        log verbose (sprintf "Found %d source files" projOptions.SourceFiles.Length)

        let allSymbols = ResizeArray<TypedSymbol>()
        let allMembers = ResizeArray<TypeMember>()

        // Type check each source file
        for sourceFile in projOptions.SourceFiles do
            if sourceFile.EndsWith(".fs") then
                log verbose (sprintf "Processing: %s" sourceFile)

                try
                    let sourceText = File.ReadAllText(sourceFile)

                    let! parseResults, checkAnswer =
                        checker.ParseAndCheckFileInProject(sourceFile, 0, SourceText.ofString sourceText, projOptions)

                    match checkAnswer with
                    | FSharpCheckFileAnswer.Succeeded checkResults ->
                        let symbols, members = extractFromCheckResults projectDir verbose checkResults
                        allSymbols.AddRange(symbols)
                        allMembers.AddRange(members)

                        if verbose then
                            for err in checkResults.Diagnostics do
                                if err.Severity = FSharpDiagnosticSeverity.Error then
                                    printfn "[error] %s" (err.ToString())

                    | FSharpCheckFileAnswer.Aborted -> log verbose (sprintf "Type checking aborted for: %s" sourceFile)
                with ex ->
                    log verbose (sprintf "Error processing %s: %s" sourceFile ex.Message)

        return (allSymbols |> Seq.toList, allMembers |> Seq.toList, Path.GetFileNameWithoutExtension(projectPath))
    }

// ============================================================================
// Main
// ============================================================================

let main (args: string[]) =
    let options = parseArgs args

    log options.Verbose (sprintf "Project: %s" options.ProjectPath)

    if not (File.Exists options.ProjectPath) then
        eprintfn "Error: Project file not found: %s" options.ProjectPath
        Environment.Exit(1)

    // Determine output path
    let projectDir = Path.GetDirectoryName(Path.GetFullPath(options.ProjectPath))

    let outputDir =
        match options.OutputPath with
        | Some p -> p
        | None -> Path.Combine(projectDir, ".fsharp-index")

    Directory.CreateDirectory(outputDir) |> ignore
    let dbPath = Path.Combine(outputDir, "index.db")

    log options.Verbose (sprintf "Database: %s" dbPath)

    // Run extraction
    let symbols, members, projectName =
        loadProject options.ProjectPath options.Verbose |> Async.RunSynchronously

    // Open/create SQLite database
    let connectionString = sprintf "Data Source=%s" dbPath
    use conn = new SqliteConnection(connectionString)
    conn.Open()

    // Initialize schema if needed
    initDatabase conn

    // Begin transaction for bulk operations
    use transaction = conn.BeginTransaction()

    try
        // Clear existing semantic data
        clearSemanticData conn

        // Update existing symbols with type information
        let mutable updatedCount = 0

        for sym in symbols do
            let updated = updateSymbolType conn sym.Qualified sym.Type

            if updated > 0 then
                updatedCount <- updatedCount + 1
            else
                // Symbol doesn't exist yet, insert it
                insertSymbol conn sym

        // Insert type members
        let distinctMembers = members |> List.distinctBy (fun m -> (m.TypeName, m.Member))

        for m in distinctMembers do
            insertMember conn m

        // Update metadata
        setMetadata conn "extracted_at" (DateTime.UtcNow.ToString("o"))
        setMetadata conn "project" projectName
        setMetadata conn "workspace_root" projectDir

        transaction.Commit()

        printfn "Updated %d symbols with type info" updatedCount
        printfn "Inserted %d new symbols" (symbols.Length - updatedCount)
        printfn "Inserted %d type members" distinctMembers.Length
        printfn "Database: %s" dbPath

        0
    with ex ->
        transaction.Rollback()
        eprintfn "Error: %s" ex.Message
        1

// Run main
let exitCode = main (fsi.CommandLineArgs |> Array.skip 1)
Environment.Exit(exitCode)

