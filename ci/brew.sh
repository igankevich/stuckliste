#!/bin/sh
set -e

brew install gnu-sed gnu-tar coreutils
prefix=/opt/homebrew
for bin in \
    "$prefix"/opt/gnu-sed/libexec/gnubin \
    "$prefix"/opt/gnu-tar/libexec/gnubin \
    "$prefix"/opt/coreutils/libexec/gnubin; do
    if ! test -e "$bin"; then
        printf "Directory %s doesn't exist.\n" "$bin" >&2
        exit 1
    fi
    printf "%s\n" "$bin" >>"$GITHUB_PATH"
done
