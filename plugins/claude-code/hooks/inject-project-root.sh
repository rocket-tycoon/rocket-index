#!/bin/bash
# inject-project-root.sh
#
# PreToolUse hook that injects Claude's cwd as project_root for RocketIndex tools.
# This bridges the gap between Claude Code's working directory and the MCP server.
#
# Input (stdin): JSON with tool_name, tool_input, cwd
# Output (stdout): JSON with hookSpecificOutput containing updatedInput

input=$(cat)

tool_name=$(echo "$input" | jq -r '.tool_name // empty')
cwd=$(echo "$input" | jq -r '.cwd // empty')
tool_input=$(echo "$input" | jq -c '.tool_input // {}')

# describe_project uses 'path' not 'project_root' - pass through unchanged
if [[ "$tool_name" == *"describe_project"* ]]; then
    echo '{"hookSpecificOutput":{"permissionDecision":"allow"}}'
    exit 0
fi

# Don't override if project_root already explicitly specified
existing=$(echo "$tool_input" | jq -r '.project_root // empty')
if [ -n "$existing" ]; then
    echo '{"hookSpecificOutput":{"permissionDecision":"allow"}}'
    exit 0
fi

# Inject cwd as project_root by merging with original input
if [ -n "$cwd" ]; then
    # Merge original tool_input with new project_root
    merged=$(echo "$tool_input" | jq -c --arg pr "$cwd" '. + {project_root: $pr}')
    echo "{\"hookSpecificOutput\":{\"permissionDecision\":\"allow\",\"updatedInput\":$merged}}"
else
    echo '{"hookSpecificOutput":{"permissionDecision":"allow"}}'
fi
