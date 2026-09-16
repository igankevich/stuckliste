#!/bin/sh
set -e
version="$(sed -rne 's/^version = "(.*)"$/\1/p' Cargo.toml)"
if test -z "$version"; then
    printf "Version not found\n" >&2
    exit 1
fi
{
    os="$(uname -s)"
    case "$os" in
    MINGW64*) os=Windows ;;
    *) ;;
    esac
    printf "VERSION=%s\n" "$version"
    printf "OS=%s\n" "$os"
    printf "ARCH=%s\n" "$(uname -m)"
} >>"$GITHUB_ENV"
