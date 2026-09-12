#!/bin/sh
# Fail-open, stderr-only advisory for whole-file source reads.

payload=
while IFS= read -r line || [ -n "$line" ]; do
    payload=$payload$line
done

case "$payload" in
    *'"tool_name"'*'"Read"'*) ;;
    *) exit 0 ;;
esac

case "$payload" in
    *'"offset"'*|*'"limit"'*|*'"start_line"'*|*'"end_line"'*) exit 0 ;;
esac

case "$payload" in
    *'.rs"'*|*'.py"'*|*'.ts"'*|*'.tsx"'*|*'.go"'*) ;;
    *) exit 0 ;;
esac

cwd=
case "$payload" in
    *'"cwd"'*)
        remainder=${payload#*'"cwd"'}
        remainder=${remainder#*:}
        remainder=${remainder#*'"'}
        cwd=${remainder%%'"'*}
        ;;
esac

if [ -z "$cwd" ]; then
    case "$payload" in
        *'"workspace_roots"'*)
            remainder=${payload#*'"workspace_roots"'}
            remainder=${remainder#*'['}
            remainder=${remainder#*'"'}
            cwd=${remainder%%'"'*}
            ;;
    esac
fi

[ -n "$cwd" ] || cwd=.

[ -f "$cwd/project.aether" ] || exit 0
printf '%s\n' 'Girder: consider `girder context . --nodes <node::path> --json --source-only` before this whole-file read.' >&2
exit 0
