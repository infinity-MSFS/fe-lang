#!/bin/sh
# Point extension.toml's grammar at a commit that actually contains
# editors/tree-sitter-fe. Zed fetches the grammar with git, so `rev` has to name
# a commit that exists in `repository` — a placeholder cannot work.
#
#   ./pin-grammar.sh            pin to HEAD on the GitHub remote (push first)
#   ./pin-grammar.sh --local    pin to HEAD in this checkout (no push needed)
#
# Run it from a shell with git on the path; on Windows, Git Bash.

set -e

manifest="$(dirname "$0")/extension.toml"
root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
rev="$(git -C "$root" rev-parse HEAD)"

if [ "$1" = "--local" ]; then
    repository="file://$root"
else
    repository="https://github.com/infinity-MSFS/fe-lang"
    if ! git -C "$root" branch -r --contains "$rev" >/dev/null 2>&1; then
        echo "warning: $rev is not on any remote branch yet — push it, or use --local" >&2
    fi
fi

sed -i.bak \
    -e "/^\[grammars.fe\]/,\$ s|^repository = .*|repository = \"$repository\"|" \
    -e "/^\[grammars.fe\]/,\$ s|^rev = .*|rev = \"$rev\"|" \
    "$manifest"
rm -f "$manifest.bak"

# Zed reuses grammars/fe if it is already there, but refuses it when its origin
# is not the repository just written — switching between --local and GitHub
# otherwise fails the next install with "failed to compile grammar 'fe'". The
# checkout is build output, so throwing it away costs one clone.
checkout="$(dirname "$0")/grammars/fe"
if [ -d "$checkout" ] &&
    [ "$(git -C "$checkout" remote get-url origin 2>/dev/null)" != "$repository" ]; then
    echo "removing grammars/fe: cloned from a different repository"
    rm -rf "$checkout" "$(dirname "$0")/grammars/fe.wasm"
fi

echo "grammar pinned to $repository @ $rev"
echo "now reinstall the dev extension in Zed so it rebuilds the grammar"
