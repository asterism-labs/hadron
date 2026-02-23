#!/usr/bin/env bash
# Claude Code PreToolUse hook — validates git commit messages before execution.
# Receives JSON on stdin with tool_name and tool_input fields.
# Outputs JSON with decision: "allow" or "deny" + reason.

set -euo pipefail

# Read stdin (JSON with tool_input)
INPUT=$(cat)

# Extract the command string from tool_input.command
CMD=$(echo "$INPUT" | python3 -c "
import sys, json
data = json.load(sys.stdin)
inp = data.get('tool_input', {})
if isinstance(inp, dict):
    print(inp.get('command', ''))
elif isinstance(inp, str):
    print(inp)
" 2>/dev/null || echo "")

# Only validate git commit commands
if ! echo "$CMD" | grep -qE '\bgit\b.*\bcommit\b'; then
    exit 0
fi

# Skip if using -F (file) or --amend without -m (editor-based)
if echo "$CMD" | grep -qE '\b-F\b|--file'; then
    exit 0
fi

# Extract message from -m flag
MSG=$(echo "$CMD" | python3 -c "
import sys, re
cmd = sys.stdin.read()
# Match -m '...' or -m \"...\" or -m word
patterns = [
    r'-m\s+\"((?:[^\"\\\\]|\\\\.)*)\"',
    r\"-m\s+'((?:[^'\\\\\\\\]|\\\\\\\\.)*)',\",
    r'-m\s+\\\$\(cat\s+<<.*?EOF\n(.*?)\nEOF',
]
for p in patterns:
    m = re.search(p, cmd, re.DOTALL)
    if m:
        print(m.group(1))
        sys.exit(0)
# Try simple -m word
m = re.search(r'-m\s+(\S+)', cmd)
if m:
    print(m.group(1))
" 2>/dev/null || echo "")

# If we can't extract a message, allow (commit-msg hook will catch it)
if [ -z "$MSG" ]; then
    exit 0
fi

SUBJECT=$(echo "$MSG" | head -n1)
VALID_TYPES="^(feat|fix|refactor|docs|test|chore|ci|perf|style|build)\([^)]+\): .+"

# Check subject format
if ! echo "$SUBJECT" | grep -qE "$VALID_TYPES"; then
    echo "{\"decision\": \"deny\", \"reason\": \"Commit subject must match type(scope): description. Got: $SUBJECT. See hadron-git-workflow skill.\"}"
    exit 0
fi

# Check subject length
SUBJECT_LEN=${#SUBJECT}
if [ "$SUBJECT_LEN" -gt 72 ]; then
    echo "{\"decision\": \"deny\", \"reason\": \"Subject must be ≤72 chars (got $SUBJECT_LEN). See hadron-git-workflow skill.\"}"
    exit 0
fi

# Check for Co-Authored-By
if echo "$MSG" | grep -qiE "^Co-Authored-By:"; then
    echo "{\"decision\": \"deny\", \"reason\": \"Co-Authored-By trailers are forbidden. See hadron-git-workflow skill.\"}"
    exit 0
fi

# All checks passed — allow
exit 0
