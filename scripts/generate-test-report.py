#!/usr/bin/env python3
"""
Generate markdown report from test-real-repos.sh JSON results.

Usage: python scripts/generate-test-report.py results/test-real-repos-<timestamp>.json
"""

import json
import sys
from collections import defaultdict
from datetime import datetime


def parse_results(results):
    """Parse raw results into structured data."""
    by_repo = defaultdict(list)
    by_test = defaultdict(list)
    by_language = defaultdict(list)

    for r in results:
        repo = r.get("repo", "unknown")
        test = r.get("test", "unknown")
        lang = repo.split("/")[0] if "/" in repo else "unknown"

        by_repo[repo].append(r)
        by_test[test].append(r)
        by_language[lang].append(r)

    return by_repo, by_test, by_language


def calculate_stats(results):
    """Calculate pass/fail/skip statistics."""
    passed = sum(1 for r in results if r.get("status") == "pass")
    failed = sum(1 for r in results if r.get("status") == "fail")
    skipped = sum(1 for r in results if r.get("status") == "skip")
    total = len(results)

    pass_rate = (passed / total * 100) if total > 0 else 0
    return {"passed": passed, "failed": failed, "skipped": skipped, "total": total, "pass_rate": pass_rate}


def generate_summary_table(by_language):
    """Generate summary table by language."""
    lines = [
        "## Summary by Language",
        "",
        "| Language | Repos | Tests | Passed | Failed | Skipped | Pass Rate |",
        "|----------|-------|-------|--------|--------|---------|-----------|",
    ]

    for lang in sorted(by_language.keys()):
        results = by_language[lang]
        stats = calculate_stats(results)

        # Count unique repos
        repos = set(r.get("repo") for r in results)

        lines.append(
            f"| {lang} | {len(repos)} | {stats['total']} | {stats['passed']} | {stats['failed']} | {stats['skipped']} | {stats['pass_rate']:.0f}% |"
        )

    return "\n".join(lines)


def generate_indexing_table(by_repo):
    """Generate indexing performance table."""
    lines = [
        "## Indexing Performance",
        "",
        "| Repo | Symbols | Time (ms) | DB Size |",
        "|------|---------|-----------|---------|",
    ]

    for repo in sorted(by_repo.keys()):
        results = by_repo[repo]
        index_result = next((r for r in results if r.get("test") == "index"), None)

        if index_result and index_result.get("status") == "pass":
            rkt_result = index_result.get("rkt_result", "")
            rkt_time = index_result.get("rkt_time_ms", "")
            notes = index_result.get("notes", "")

            # Extract db_size from notes
            db_size = ""
            if "db_size:" in notes:
                db_size = notes.split("db_size:")[1].strip()

            lines.append(f"| {repo} | {rkt_result} | {rkt_time} | {db_size} |")

    return "\n".join(lines)


def generate_comparison_table(by_repo):
    """Generate RocketIndex vs grep comparison table."""
    lines = [
        "## RocketIndex vs grep Comparison",
        "",
        "| Repo | Test | RocketIndex | grep | Precision Gain |",
        "|------|------|-------------|------|----------------|",
    ]

    for repo in sorted(by_repo.keys()):
        results = by_repo[repo]

        for r in results:
            test = r.get("test", "")
            if test not in ["def", "callers"]:
                continue

            rkt_result = r.get("rkt_result", "0")
            grep_result = r.get("grep_result", "0")

            # Extract numbers
            try:
                rkt_num = int("".join(c for c in rkt_result.split()[0] if c.isdigit()) or "0")
                grep_num = int("".join(c for c in grep_result.split()[0] if c.isdigit()) or "0")
            except (ValueError, IndexError):
                rkt_num = 0
                grep_num = 0

            # Calculate precision gain
            if rkt_num > 0 and grep_num > rkt_num:
                gain = f"{grep_num / rkt_num:.1f}x"
            elif rkt_num > 0 and grep_num == rkt_num:
                gain = "1x (equal)"
            elif rkt_num == 0:
                gain = "N/A"
            else:
                gain = "1x"

            lines.append(f"| {repo} | {test} | {rkt_result} | {grep_result} | {gain} |")

    return "\n".join(lines)


def generate_issues_section(results):
    """Generate issues discovered section."""
    lines = ["## Issues Discovered", ""]

    failures = [r for r in results if r.get("status") == "fail"]

    if not failures:
        lines.append("No failures detected.")
        return "\n".join(lines)

    for f in failures:
        repo = f.get("repo", "unknown")
        test = f.get("test", "unknown")
        notes = f.get("notes", "")
        lines.append(f"- **{repo}** ({test}): {notes}")

    return "\n".join(lines)


def generate_tool_results_table(by_repo):
    """Generate per-repo tool results matrix."""
    lines = [
        "## Tool Results by Repository",
        "",
        "| Repo | def | callers | refs | spider | spider_rev | symbols | subclasses |",
        "|------|-----|---------|------|--------|------------|---------|------------|",
    ]

    tests = ["def", "callers", "refs", "spider", "spider_reverse", "symbols", "subclasses"]

    for repo in sorted(by_repo.keys()):
        results = by_repo[repo]
        result_map = {r.get("test"): r.get("status", "-") for r in results}

        row = [repo]
        for test in tests:
            status = result_map.get(test, "-")
            if status == "pass":
                row.append("PASS")
            elif status == "fail":
                row.append("FAIL")
            elif status == "skip":
                row.append("skip")
            else:
                row.append("-")

        lines.append("| " + " | ".join(row) + " |")

    return "\n".join(lines)


def generate_report(results_file):
    """Generate full markdown report."""
    with open(results_file, "r") as f:
        results = json.load(f)

    by_repo, by_test, by_language = parse_results(results)

    # Calculate overall stats
    overall_stats = calculate_stats(results)

    report = [
        "# RocketIndex Test Results",
        "",
        f"Generated: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}",
        "",
        "## Overall Statistics",
        "",
        f"- **Total Tests**: {overall_stats['total']}",
        f"- **Passed**: {overall_stats['passed']}",
        f"- **Failed**: {overall_stats['failed']}",
        f"- **Skipped**: {overall_stats['skipped']}",
        f"- **Pass Rate**: {overall_stats['pass_rate']:.1f}%",
        "",
        "---",
        "",
        generate_summary_table(by_language),
        "",
        "---",
        "",
        generate_tool_results_table(by_repo),
        "",
        "---",
        "",
        generate_indexing_table(by_repo),
        "",
        "---",
        "",
        generate_comparison_table(by_repo),
        "",
        "---",
        "",
        generate_issues_section(results),
    ]

    return "\n".join(report)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("Usage: python generate-test-report.py <results.json>", file=sys.stderr)
        sys.exit(1)

    results_file = sys.argv[1]
    print(generate_report(results_file))
