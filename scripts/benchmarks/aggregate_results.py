#!/usr/bin/env python3
"""
aggregate_results.py - Generate markdown tables from benchmark results

Usage:
    python3 aggregate_results.py --input results/ --output docs/benchmarks/results.md
    python3 aggregate_results.py --input results/ --format table  # stdout only
"""

import argparse
import json
import os
from pathlib import Path
from collections import defaultdict
from datetime import datetime


def load_results(input_dir: str) -> list[dict]:
    """Load all JSON result files from the input directory."""
    results = []
    input_path = Path(input_dir)

    for json_file in input_path.glob("*.json"):
        # Skip raw output files (those with _run in the name)
        if "_run" in json_file.name:
            continue
        try:
            with open(json_file) as f:
                data = json.load(f)
                data["_file"] = json_file.name
                results.append(data)
        except json.JSONDecodeError as e:
            print(f"Warning: Could not parse {json_file}: {e}")

    return results


def group_by_language_model(results: list[dict]) -> dict:
    """Group results by language and model."""
    grouped = defaultdict(lambda: defaultdict(list))

    for result in results:
        # Extract language from filename (e.g., ruby_sonnet_find_callers_...)
        filename = result.get("_file", "")
        parts = filename.split("_")
        if len(parts) >= 2:
            language = parts[0]
            model = result.get("model", parts[1] if len(parts) > 1 else "unknown")
        else:
            language = "unknown"
            model = result.get("model", "unknown")

        grouped[language][model].append(result)

    return grouped


def generate_summary_table(grouped: dict) -> str:
    """Generate the main summary table."""
    lines = [
        "## Summary",
        "",
        "| Language | Model | Avg Turn Reduction | Success Rate (with RKT) | Tasks |",
        "|----------|-------|-------------------|-------------------------|-------|",
    ]

    for language in sorted(grouped.keys()):
        for model in sorted(grouped[language].keys()):
            tasks = grouped[language][model]

            # Calculate averages
            turn_reductions = [t.get("turn_reduction_percent", 0) for t in tasks]
            avg_reduction = sum(turn_reductions) / len(turn_reductions) if turn_reductions else 0

            success_rates = [t.get("with_rkt", {}).get("success_rate", 0) for t in tasks]
            avg_success = sum(success_rates) / len(success_rates) if success_rates else 0

            task_count = len(tasks)

            lines.append(
                f"| {language.title()} | {model.title()} | {avg_reduction:+.0f}% | {avg_success*100:.0f}% | {task_count} |"
            )

    return "\n".join(lines)


def generate_detail_tables(grouped: dict) -> str:
    """Generate detailed per-language tables."""
    sections = []

    for language in sorted(grouped.keys()):
        section_lines = [
            f"## {language.title()} Results",
            "",
        ]

        for model in sorted(grouped[language].keys()):
            tasks = grouped[language][model]

            section_lines.extend([
                f"### {model.title()}",
                "",
                "| Task | Category | Without RKT | With RKT | Turn Reduction |",
                "|------|----------|-------------|----------|----------------|",
            ])

            for task in sorted(tasks, key=lambda t: t.get("task_id", "")):
                task_id = task.get("task_id", "unknown")
                category = task.get("category", "unknown")

                without = task.get("without_rkt", {})
                with_rkt = task.get("with_rkt", {})

                without_turns = without.get("avg_turns", "N/A")
                without_success = without.get("success_rate", 0)
                with_turns = with_rkt.get("avg_turns", "N/A")
                with_success = with_rkt.get("success_rate", 0)

                reduction = task.get("turn_reduction_percent", 0)

                # Format with success indicators
                without_str = f"{without_turns} turns"
                if without_success < 1.0:
                    without_str += f" ({without_success*100:.0f}% success)"

                with_str = f"{with_turns} turns"
                if with_success < 1.0:
                    with_str += f" ({with_success*100:.0f}% success)"

                section_lines.append(
                    f"| {task_id} | {category} | {without_str} | {with_str} | {reduction:+.0f}% |"
                )

            section_lines.append("")

        sections.append("\n".join(section_lines))

    return "\n".join(sections)


def generate_findings(grouped: dict) -> str:
    """Generate key findings section."""
    lines = [
        "## Key Findings",
        "",
    ]

    # Find best Haiku improvement
    haiku_improvements = []
    for language, models in grouped.items():
        if "haiku" in models:
            for task in models["haiku"]:
                reduction = task.get("turn_reduction_percent", 0)
                task_id = task.get("task_id", "unknown")
                haiku_improvements.append((reduction, language, task_id))

    if haiku_improvements:
        best = max(haiku_improvements, key=lambda x: x[0])
        lines.extend([
            f"### Haiku Model Uplift",
            f"- Best improvement: **{best[0]:+.0f}%** turn reduction on `{best[2]}` ({best[1]})",
            f"- RocketIndex enables Haiku to complete tasks it would otherwise fail",
            "",
        ])

    # Find tasks where without_rkt failed
    failures_prevented = []
    for language, models in grouped.items():
        for model, tasks in models.items():
            for task in tasks:
                without = task.get("without_rkt", {})
                with_rkt = task.get("with_rkt", {})
                if without.get("success_rate", 1) < 0.5 and with_rkt.get("success_rate", 0) >= 0.5:
                    failures_prevented.append((language, model, task.get("task_id", "")))

    if failures_prevented:
        lines.extend([
            "### Failure Prevention",
            f"- RocketIndex prevented {len(failures_prevented)} task failure(s):",
        ])
        for lang, model, task_id in failures_prevented[:5]:  # Limit to 5
            lines.append(f"  - {lang}/{model}: `{task_id}`")
        lines.append("")

    return "\n".join(lines)


def generate_markdown(results: list[dict], include_timestamp: bool = True) -> str:
    """Generate the full markdown report."""
    grouped = group_by_language_model(results)

    sections = [
        "# RocketIndex Benchmark Results",
        "",
    ]

    if include_timestamp:
        sections.extend([
            f"*Generated: {datetime.now().strftime('%Y-%m-%d %H:%M')}*",
            "",
        ])

    sections.extend([
        generate_summary_table(grouped),
        "",
        generate_findings(grouped),
        "",
        generate_detail_tables(grouped),
        "",
        "## Reproduction",
        "",
        "```bash",
        "# Index the repository first",
        "cd /path/to/repo && rkt index",
        "",
        "# Run benchmarks",
        "./scripts/benchmarks/run_benchmark.sh \\",
        "  --task-file tasks/ruby_vets_api.json \\",
        "  --model haiku",
        "",
        "# Aggregate results",
        "python3 scripts/benchmarks/aggregate_results.py \\",
        "  --input scripts/benchmarks/results/ \\",
        "  --output docs/benchmarks/results.md",
        "```",
    ])

    return "\n".join(sections)


def main():
    parser = argparse.ArgumentParser(description="Aggregate benchmark results into markdown")
    parser.add_argument("--input", "-i", required=True, help="Directory containing result JSON files")
    parser.add_argument("--output", "-o", help="Output markdown file (default: stdout)")
    parser.add_argument("--format", choices=["markdown", "table"], default="markdown",
                        help="Output format")

    args = parser.parse_args()

    results = load_results(args.input)

    if not results:
        print(f"No results found in {args.input}")
        return 1

    print(f"Loaded {len(results)} result files")

    if args.format == "table":
        grouped = group_by_language_model(results)
        output = generate_summary_table(grouped)
    else:
        output = generate_markdown(results)

    if args.output:
        output_path = Path(args.output)
        output_path.parent.mkdir(parents=True, exist_ok=True)
        with open(output_path, "w") as f:
            f.write(output)
        print(f"Written to {args.output}")
    else:
        print(output)

    return 0


if __name__ == "__main__":
    exit(main())
