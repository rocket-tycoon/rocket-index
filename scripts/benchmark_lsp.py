#!/usr/bin/env python3
"""
LSP Performance Benchmark

Compares fsharp-lsp vs fsautocomplete on common operations:
- Startup time
- Go to definition
- Workspace symbol search
- Find references
- Memory usage

Usage:
    python3 benchmark_lsp.py --project /path/to/fsharp/project
    python3 benchmark_lsp.py --project /path/to/fsharp/project --fsautocomplete /path/to/fsautocomplete
"""

import argparse
import json
import os
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional


@dataclass
class BenchmarkResult:
    """Results from a single benchmark run."""
    operation: str
    lsp_name: str
    duration_ms: float
    success: bool
    details: str = ""


@dataclass
class LSPClient:
    """Simple LSP client for benchmarking."""
    process: subprocess.Popen
    name: str
    request_id: int = 0

    def send_request(self, method: str, params: dict) -> tuple[dict, float]:
        """Send a request and return (response, duration_ms)."""
        self.request_id += 1
        request = {
            "jsonrpc": "2.0",
            "id": self.request_id,
            "method": method,
            "params": params
        }

        content = json.dumps(request)
        message = f"Content-Length: {len(content)}\r\n\r\n{content}"

        start = time.perf_counter()
        self.process.stdin.write(message.encode())
        self.process.stdin.flush()

        response = self._read_response()
        duration_ms = (time.perf_counter() - start) * 1000

        return response, duration_ms

    def send_notification(self, method: str, params: dict):
        """Send a notification (no response expected)."""
        notification = {
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }

        content = json.dumps(notification)
        message = f"Content-Length: {len(content)}\r\n\r\n{content}"

        self.process.stdin.write(message.encode())
        self.process.stdin.flush()

    def _read_response(self) -> dict:
        """Read a JSON-RPC response from stdout."""
        # Read headers
        headers = {}
        while True:
            line = self.process.stdout.readline().decode('utf-8')
            if line == '\r\n' or line == '\n':
                break
            if ':' in line:
                key, value = line.split(':', 1)
                headers[key.strip().lower()] = value.strip()

        # Read content
        content_length = int(headers.get('content-length', 0))
        if content_length > 0:
            content = self.process.stdout.read(content_length).decode('utf-8')
            return json.loads(content)
        return {}

    def get_memory_mb(self) -> float:
        """Get current memory usage in MB."""
        try:
            import psutil
            proc = psutil.Process(self.process.pid)
            return proc.memory_info().rss / (1024 * 1024)
        except ImportError:
            # Fallback: try /proc on Linux/macOS
            try:
                with open(f"/proc/{self.process.pid}/status") as f:
                    for line in f:
                        if line.startswith("VmRSS:"):
                            return int(line.split()[1]) / 1024
            except:
                pass
            return -1

    def shutdown(self):
        """Shutdown the LSP server."""
        try:
            self.send_request("shutdown", {})
            self.send_notification("exit", {})
        except:
            pass
        self.process.terminate()
        self.process.wait(timeout=5)


def start_lsp(command: list[str], name: str) -> tuple[LSPClient, float]:
    """Start an LSP server and return (client, startup_time_ms)."""
    start = time.perf_counter()

    process = subprocess.Popen(
        command,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )

    client = LSPClient(process=process, name=name)

    # Wait a moment for process to start
    time.sleep(0.1)

    startup_ms = (time.perf_counter() - start) * 1000
    return client, startup_ms


def initialize_lsp(client: LSPClient, workspace_path: str) -> tuple[dict, float]:
    """Send initialize request and return (result, duration_ms)."""
    params = {
        "processId": os.getpid(),
        "rootUri": f"file://{workspace_path}",
        "rootPath": workspace_path,
        "capabilities": {
            "textDocument": {
                "definition": {"dynamicRegistration": False},
                "references": {"dynamicRegistration": False},
                "hover": {"dynamicRegistration": False},
                "completion": {"dynamicRegistration": False},
            },
            "workspace": {
                "symbol": {"dynamicRegistration": False},
            }
        },
        "workspaceFolders": [
            {"uri": f"file://{workspace_path}", "name": os.path.basename(workspace_path)}
        ]
    }

    response, duration = client.send_request("initialize", params)

    # Send initialized notification
    client.send_notification("initialized", {})

    # Give the server time to index
    time.sleep(0.5)

    return response, duration


def find_test_file(workspace: Path) -> Optional[Path]:
    """Find a suitable F# file for testing."""
    for pattern in ["**/*.fs", "**/*.fsx"]:
        files = list(workspace.glob(pattern))
        # Prefer files with more content
        files.sort(key=lambda f: f.stat().st_size, reverse=True)
        for f in files:
            if not f.name.startswith("."):
                return f
    return None


def find_symbol_position(file_path: Path) -> tuple[int, int]:
    """Find a symbol position in a file for testing go-to-definition."""
    content = file_path.read_text()
    lines = content.split('\n')

    # Look for a function call or identifier
    for i, line in enumerate(lines):
        # Skip comments and empty lines
        stripped = line.strip()
        if not stripped or stripped.startswith("//") or stripped.startswith("(*"):
            continue

        # Look for identifiers after 'let', 'open', function calls, etc.
        for keyword in ['let ', 'open ', 'type ', 'module ']:
            if keyword in line:
                col = line.find(keyword) + len(keyword)
                # Find the end of the identifier
                while col < len(line) and (line[col].isalnum() or line[col] == '_'):
                    col += 1
                if col > line.find(keyword) + len(keyword):
                    return (i, line.find(keyword) + len(keyword))

    # Fallback: first non-empty, non-comment line
    for i, line in enumerate(lines):
        stripped = line.strip()
        if stripped and not stripped.startswith("//"):
            return (i, 0)

    return (0, 0)


def benchmark_goto_definition(client: LSPClient, file_path: Path, line: int, col: int) -> BenchmarkResult:
    """Benchmark go-to-definition."""
    # Open the document first
    content = file_path.read_text()
    client.send_notification("textDocument/didOpen", {
        "textDocument": {
            "uri": f"file://{file_path}",
            "languageId": "fsharp",
            "version": 1,
            "text": content
        }
    })

    # Small delay for document to be processed
    time.sleep(0.1)

    params = {
        "textDocument": {"uri": f"file://{file_path}"},
        "position": {"line": line, "character": col}
    }

    response, duration = client.send_request("textDocument/definition", params)

    success = "result" in response and response["result"] is not None
    details = f"line {line}, col {col}"
    if not success and "error" in response:
        details += f" - {response['error'].get('message', 'unknown error')}"

    return BenchmarkResult(
        operation="go-to-definition",
        lsp_name=client.name,
        duration_ms=duration,
        success=success,
        details=details
    )


def benchmark_workspace_symbol(client: LSPClient, query: str) -> BenchmarkResult:
    """Benchmark workspace symbol search."""
    params = {"query": query}

    response, duration = client.send_request("workspace/symbol", params)

    success = "result" in response
    result_count = len(response.get("result", []) or [])
    details = f"query='{query}', found {result_count} symbols"

    return BenchmarkResult(
        operation="workspace-symbol",
        lsp_name=client.name,
        duration_ms=duration,
        success=success,
        details=details
    )


def benchmark_references(client: LSPClient, file_path: Path, line: int, col: int) -> BenchmarkResult:
    """Benchmark find references."""
    params = {
        "textDocument": {"uri": f"file://{file_path}"},
        "position": {"line": line, "character": col},
        "context": {"includeDeclaration": True}
    }

    response, duration = client.send_request("textDocument/references", params)

    success = "result" in response
    result_count = len(response.get("result", []) or [])
    details = f"found {result_count} references"

    return BenchmarkResult(
        operation="find-references",
        lsp_name=client.name,
        duration_ms=duration,
        success=success,
        details=details
    )


def benchmark_hover(client: LSPClient, file_path: Path, line: int, col: int) -> BenchmarkResult:
    """Benchmark hover information."""
    params = {
        "textDocument": {"uri": f"file://{file_path}"},
        "position": {"line": line, "character": col}
    }

    response, duration = client.send_request("textDocument/hover", params)

    success = "result" in response and response["result"] is not None
    details = "got hover info" if success else "no hover info"

    return BenchmarkResult(
        operation="hover",
        lsp_name=client.name,
        duration_ms=duration,
        success=success,
        details=details
    )


def benchmark_completion(client: LSPClient, file_path: Path, line: int, col: int) -> BenchmarkResult:
    """Benchmark completion."""
    params = {
        "textDocument": {"uri": f"file://{file_path}"},
        "position": {"line": line, "character": col}
    }

    response, duration = client.send_request("textDocument/completion", params)

    success = "result" in response
    result = response.get("result")
    if isinstance(result, list):
        result_count = len(result)
    elif isinstance(result, dict):
        result_count = len(result.get("items", []))
    else:
        result_count = 0
    details = f"got {result_count} completions"

    return BenchmarkResult(
        operation="completion",
        lsp_name=client.name,
        duration_ms=duration,
        success=success,
        details=details
    )


def run_benchmarks(
    lsp_command: list[str],
    lsp_name: str,
    workspace: Path,
    iterations: int = 3
) -> list[BenchmarkResult]:
    """Run all benchmarks for a single LSP."""
    results = []

    print(f"\n{'='*60}")
    print(f"Benchmarking: {lsp_name}")
    print(f"{'='*60}")

    # Find test file
    test_file = find_test_file(workspace)
    if not test_file:
        print(f"  ERROR: No F# files found in {workspace}")
        return results

    print(f"  Test file: {test_file.name}")
    line, col = find_symbol_position(test_file)
    print(f"  Test position: line {line}, col {col}")

    # Start LSP and measure startup
    print(f"\n  Starting LSP...")
    try:
        client, startup_ms = start_lsp(lsp_command, lsp_name)
    except Exception as e:
        print(f"  ERROR: Failed to start LSP: {e}")
        return results

    results.append(BenchmarkResult(
        operation="process-start",
        lsp_name=lsp_name,
        duration_ms=startup_ms,
        success=True,
        details="process spawned"
    ))
    print(f"  Process start: {startup_ms:.1f}ms")

    # Initialize
    print(f"  Initializing...")
    try:
        _, init_ms = initialize_lsp(client, str(workspace))
        results.append(BenchmarkResult(
            operation="initialize",
            lsp_name=lsp_name,
            duration_ms=init_ms,
            success=True,
            details="initialized + indexed"
        ))
        print(f"  Initialize: {init_ms:.1f}ms")
    except Exception as e:
        print(f"  ERROR: Failed to initialize: {e}")
        client.shutdown()
        return results

    # Wait for indexing to complete
    print(f"  Waiting for index to build...")
    time.sleep(2)

    # Memory after init
    mem_mb = client.get_memory_mb()
    if mem_mb > 0:
        results.append(BenchmarkResult(
            operation="memory-after-init",
            lsp_name=lsp_name,
            duration_ms=mem_mb,  # Abusing duration_ms for memory
            success=True,
            details=f"{mem_mb:.1f} MB"
        ))
        print(f"  Memory after init: {mem_mb:.1f} MB")

    # Run benchmarks multiple times
    for i in range(iterations):
        print(f"\n  --- Iteration {i+1}/{iterations} ---")

        # Go to definition
        result = benchmark_goto_definition(client, test_file, line, col)
        results.append(result)
        status = "OK" if result.success else "FAIL"
        print(f"  Go to definition: {result.duration_ms:.2f}ms [{status}] ({result.details})")

        # Workspace symbol - prefix search
        result = benchmark_workspace_symbol(client, "get")
        results.append(result)
        status = "OK" if result.success else "FAIL"
        print(f"  Workspace symbol (prefix): {result.duration_ms:.2f}ms [{status}] ({result.details})")

        # Workspace symbol - contains search
        result = benchmark_workspace_symbol(client, "Service")
        results.append(result)
        status = "OK" if result.success else "FAIL"
        print(f"  Workspace symbol (contains): {result.duration_ms:.2f}ms [{status}] ({result.details})")

        # Find references
        result = benchmark_references(client, test_file, line, col)
        results.append(result)
        status = "OK" if result.success else "FAIL"
        print(f"  Find references: {result.duration_ms:.2f}ms [{status}] ({result.details})")

        # Hover
        result = benchmark_hover(client, test_file, line, col)
        results.append(result)
        status = "OK" if result.success else "FAIL"
        print(f"  Hover: {result.duration_ms:.2f}ms [{status}] ({result.details})")

        # Completion
        result = benchmark_completion(client, test_file, line, col)
        results.append(result)
        status = "OK" if result.success else "FAIL"
        print(f"  Completion: {result.duration_ms:.2f}ms [{status}] ({result.details})")

    # Final memory
    mem_mb = client.get_memory_mb()
    if mem_mb > 0:
        results.append(BenchmarkResult(
            operation="memory-final",
            lsp_name=lsp_name,
            duration_ms=mem_mb,
            success=True,
            details=f"{mem_mb:.1f} MB"
        ))
        print(f"\n  Final memory: {mem_mb:.1f} MB")

    # Shutdown
    print(f"  Shutting down...")
    client.shutdown()

    return results


def print_summary(all_results: list[BenchmarkResult]):
    """Print a summary comparison table."""
    print(f"\n{'='*80}")
    print("BENCHMARK SUMMARY")
    print(f"{'='*80}")

    # Group by operation
    operations = {}
    for result in all_results:
        key = result.operation
        if key not in operations:
            operations[key] = {}
        if result.lsp_name not in operations[key]:
            operations[key][result.lsp_name] = []
        operations[key][result.lsp_name].append(result.duration_ms)

    # Print table
    print(f"\n{'Operation':<25} {'fsharp-lsp':<20} {'fsautocomplete':<20} {'Speedup':<10}")
    print("-" * 75)

    for op, lsps in sorted(operations.items()):
        fsharp_times = lsps.get("fsharp-lsp", [])
        fsauto_times = lsps.get("fsautocomplete", [])

        fsharp_avg = sum(fsharp_times) / len(fsharp_times) if fsharp_times else 0
        fsauto_avg = sum(fsauto_times) / len(fsauto_times) if fsauto_times else 0

        fsharp_str = f"{fsharp_avg:.2f}ms" if fsharp_times else "N/A"
        fsauto_str = f"{fsauto_avg:.2f}ms" if fsauto_times else "N/A"

        if fsharp_avg > 0 and fsauto_avg > 0:
            if op.startswith("memory"):
                # For memory, lower is better
                speedup = fsauto_avg / fsharp_avg
                speedup_str = f"{speedup:.1f}x less"
            else:
                speedup = fsauto_avg / fsharp_avg
                speedup_str = f"{speedup:.1f}x faster" if speedup > 1 else f"{1/speedup:.1f}x slower"
        else:
            speedup_str = "N/A"

        print(f"{op:<25} {fsharp_str:<20} {fsauto_str:<20} {speedup_str:<10}")


def main():
    parser = argparse.ArgumentParser(description="Benchmark LSP performance")
    parser.add_argument("--project", "-p", required=True, help="Path to F# project/workspace")
    parser.add_argument("--fsharp-lsp", default=None, help="Path to fsharp-lsp binary")
    parser.add_argument("--fsautocomplete", default=None, help="Path to fsautocomplete binary")
    parser.add_argument("--iterations", "-n", type=int, default=3, help="Number of iterations per benchmark")
    parser.add_argument("--only", choices=["fsharp-lsp", "fsautocomplete"], help="Only benchmark one LSP")

    args = parser.parse_args()

    workspace = Path(args.project).resolve()
    if not workspace.exists():
        print(f"ERROR: Project path does not exist: {workspace}")
        sys.exit(1)

    print(f"Workspace: {workspace}")

    # Find LSP binaries
    fsharp_lsp_path = args.fsharp_lsp
    if not fsharp_lsp_path:
        # Try to find in typical locations
        candidates = [
            Path(__file__).parent.parent / "target" / "release" / "fsharp-lsp",
            Path.home() / ".cargo" / "bin" / "fsharp-lsp",
        ]
        for c in candidates:
            if c.exists():
                fsharp_lsp_path = str(c)
                break

    fsauto_path = args.fsautocomplete
    if not fsauto_path:
        # Try to find fsautocomplete
        candidates = [
            Path.home() / ".dotnet" / "tools" / "fsautocomplete",
            Path("/usr/local/bin/fsautocomplete"),
        ]
        for c in candidates:
            if c.exists():
                fsauto_path = str(c)
                break

    all_results = []

    # Benchmark fsharp-lsp
    if args.only != "fsautocomplete":
        if fsharp_lsp_path and Path(fsharp_lsp_path).exists():
            results = run_benchmarks(
                [fsharp_lsp_path],
                "fsharp-lsp",
                workspace,
                args.iterations
            )
            all_results.extend(results)
        else:
            print(f"\nWARNING: fsharp-lsp not found. Use --fsharp-lsp to specify path.")
            print(f"  Tried: {fsharp_lsp_path}")

    # Benchmark fsautocomplete
    if args.only != "fsharp-lsp":
        if fsauto_path and Path(fsauto_path).exists():
            results = run_benchmarks(
                [fsauto_path, "--adaptive-lsp-server-enabled"],
                "fsautocomplete",
                workspace,
                args.iterations
            )
            all_results.extend(results)
        else:
            print(f"\nWARNING: fsautocomplete not found. Use --fsautocomplete to specify path.")
            print(f"  Install with: dotnet tool install -g fsautocomplete")

    # Print summary
    if all_results:
        print_summary(all_results)

    # Output JSON for further analysis
    json_output = Path(workspace) / ".fsharp-index" / "benchmark-results.json"
    json_output.parent.mkdir(exist_ok=True)
    with open(json_output, "w") as f:
        json.dump([{
            "operation": r.operation,
            "lsp": r.lsp_name,
            "duration_ms": r.duration_ms,
            "success": r.success,
            "details": r.details
        } for r in all_results], f, indent=2)
    print(f"\nDetailed results saved to: {json_output}")


if __name__ == "__main__":
    main()
