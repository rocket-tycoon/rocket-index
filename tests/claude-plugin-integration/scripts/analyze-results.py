#!/usr/bin/env python3
"""
Analyze Claude plugin test results for quality and correctness
"""

import json
import glob
from pathlib import Path
from typing import Dict, List

def analyze_result(result_file: Path) -> Dict:
    """Analyze a single test result JSON"""
    lang = result_file.stem.replace('_test', '')
    
    try:
        with open(result_file) as f:
            data = json.load(f)
    except json.JSONDecodeError:
        # Fallback: read as text if JSON parsing fails
        with open(result_file) as f:
            content = f.read()
            return {
                'language': lang,
                'status': 'parse_error',
                'raw_length': len(content),
                'tool_usage': 'unknown'
            }
    
    # Extract metrics
    metrics = {
        'language': lang,
        'status': 'success',
        'tool_mentions': count_tool_mentions(data),
        'grep_mentions': count_grep_mentions(data),
        'quality_score': 'manual_review_required'
    }
    
    return metrics

def count_tool_mentions(data) -> int:
    """Count mentions of Rocket Index MCP tools"""
    text = json.dumps(data).lower()
    tools = ['find_definition', 'find_callers', 'find_references', 
             'search_symbols', 'analyze_dependencies']
    return sum(text.count(tool) for tool in tools)

def count_grep_mentions(data) -> int:
    """Count mentions of grep (fallback indicator)"""
    text = json.dumps(data).lower()
    return text.count('grep')

def main():
    results = []
    
    results_dir = Path(__file__).parent.parent / 'results'
    
    print("Analyzing test results...\n")
    
    for result_file in sorted(results_dir.glob('*_test.json')):
        metrics = analyze_result(result_file)
        results.append(metrics)
    
    # Print summary table
    print("Language      | Status  | Tool Mentions | Grep Mentions")
    print("--------------|---------|--------------|--------------")
    for r in results:
        print(f"{r['language']:13} | {r['status']:7} | {r.get('tool_mentions', 0):12} | {r.get('grep_mentions', 0):13}")
    
    print(f"\nTotal languages tested: {len(results)}")
    print("\nFor detailed analysis, review individual JSON files in results/")

if __name__ == '__main__':
    main()
