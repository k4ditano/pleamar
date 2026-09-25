#!/bin/sh
# The guard of the cross-platform approach: the core has to compile for the
# three systems, even if today only Linux has a platform. If this fails,
# something system-specific has slipped outside src/platform/.
#
# Luau is compiled from its C++ sources, and for another system that asks for
# a cross compiler. If there is none, that system is checked without the Luau
# logic —which is Rust with nothing system-specific— and it says so.
set -e
native=$(rustc -vV | sed -n 's/^host: //p')
check() {
    cargo check --quiet --target "$1" $2 2>&1 | grep -E "^error" | head -3 | grep . && exit 1
    return 0
}
for t in x86_64-unknown-linux-gnu x86_64-pc-windows-gnu aarch64-apple-darwin; do
    printf '%-28s ' "$t"
    cxx=""
    case "$t" in
        *windows-gnu) cxx=x86_64-w64-mingw32-g++ ;;
        *apple-darwin) cxx=o64-clang++ ;;
    esac
    if [ "$t" = "$native" ] || command -v "$cxx" >/dev/null 2>&1; then
        check "$t" "" && echo "ok"
    else
        check "$t" "--no-default-features" && echo "ok (without Luau: there is no $cxx here)"
    fi
done
