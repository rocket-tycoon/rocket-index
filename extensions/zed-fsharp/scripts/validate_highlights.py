#!/usr/bin/env python3
"""
Validate highlights.scm against tree-sitter grammar node-types.json.

Checks that all node types and literals referenced in highlights.scm
actually exist in the grammar.
"""

import json
import re
import sys
from pathlib import Path


def load_node_types(node_types_path: Path) -> tuple[set[str], set[str]]:
    """Load node-types.json and return (named_nodes, anonymous_nodes)."""
    with open(node_types_path) as f:
        data = json.load(f)

    named = set()
    anonymous = set()

    for node in data:
        node_type = node.get("type", "")
        if node.get("named", False):
            named.add(node_type)
        else:
            anonymous.add(node_type)

    return named, anonymous


def remove_predicates(content: str) -> str:
    """Remove all predicate expressions, replacing with spaces to preserve positions."""
    result = list(content)
    i = 0
    while i < len(content):
        # Check for predicate start: (#
        if i + 1 < len(content) and content[i] == '(' and content[i + 1] == '#':
            start = i
            # Skip until we find matching closing paren
            depth = 1
            i += 2
            while i < len(content) and depth > 0:
                if content[i] == '(':
                    depth += 1
                elif content[i] == ')':
                    depth -= 1
                i += 1
            # Replace predicate with spaces (preserve newlines for line tracking)
            for j in range(start, i):
                if result[j] != '\n':
                    result[j] = ' '
        else:
            i += 1
    return ''.join(result)


def extract_references(highlights_path: Path) -> list[tuple[int, str, str]]:
    """
    Extract node references from highlights.scm.
    Returns list of (line_number, reference_type, value).
    reference_type is 'node' or 'literal'.
    """
    with open(highlights_path) as f:
        content = f.read()
        lines = content.split('\n')

    # Remove comments (preserve positions)
    content_no_comments = re.sub(r';[^\n]*', lambda m: ' ' * len(m.group()), content)

    # Remove predicates (preserve positions)
    content_clean = remove_predicates(content_no_comments)

    # Now process line by line for accurate line numbers
    references = []
    clean_lines = content_clean.split('\n')

    for line_num, line in enumerate(clean_lines, 1):
        # Find named node references: (node_name)
        # Must start with lowercase letter (not _ which is wildcard)
        node_pattern = re.compile(r'\(([a-z][a-z0-9_]*)')
        for match in node_pattern.finditer(line):
            node_name = match.group(1)
            references.append((line_num, 'node', node_name))

        # Find supertype references like (_type), (_pattern), (_expression)
        supertype_pattern = re.compile(r'\((_[a-z][a-z0-9_]*)')
        for match in supertype_pattern.finditer(line):
            node_name = match.group(1)
            references.append((line_num, 'node', node_name))

        # Find literal references: "something"
        # Must be a simple quoted string (not multiline)
        literal_pattern = re.compile(r'"([^"\n]+)"')
        for match in literal_pattern.finditer(line):
            literal = match.group(1)
            # Skip if it looks like leftover from predicate removal
            if literal.strip() == '':
                continue
            references.append((line_num, 'literal', literal))

    return references


def validate(highlights_path: Path, node_types_path: Path) -> list[str]:
    """Validate highlights.scm against node-types.json. Returns list of errors."""
    named_nodes, anonymous_nodes = load_node_types(node_types_path)
    references = extract_references(highlights_path)

    errors = []

    for line_num, ref_type, value in references:
        if ref_type == 'node':
            if value not in named_nodes:
                errors.append(f"Line {line_num}: Invalid node type '{value}'")
        elif ref_type == 'literal':
            if value not in anonymous_nodes:
                errors.append(f"Line {line_num}: Invalid literal '{value}'")

    return errors


def main():
    # Default paths relative to script location
    script_dir = Path(__file__).parent
    ext_dir = script_dir.parent

    highlights_path = ext_dir / "languages" / "fsharp" / "highlights.scm"
    node_types_path = ext_dir / "grammars" / "fsharp" / "fsharp" / "src" / "node-types.json"

    # Allow overriding paths via command line
    if len(sys.argv) > 1:
        highlights_path = Path(sys.argv[1])
    if len(sys.argv) > 2:
        node_types_path = Path(sys.argv[2])

    if not highlights_path.exists():
        print(f"Error: highlights.scm not found at {highlights_path}")
        sys.exit(1)

    if not node_types_path.exists():
        print(f"Error: node-types.json not found at {node_types_path}")
        sys.exit(1)

    print(f"Validating: {highlights_path}")
    print(f"Against: {node_types_path}")
    print()

    errors = validate(highlights_path, node_types_path)

    if errors:
        print(f"Found {len(errors)} error(s):")
        for error in errors:
            print(f"  {error}")
        sys.exit(1)
    else:
        print("All node types and literals are valid!")
        sys.exit(0)


if __name__ == "__main__":
    main()
