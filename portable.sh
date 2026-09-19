#!/bin/sh
# La guarda del enfoque multiplataforma: el núcleo tiene que compilar para los
# tres sistemas, aunque hoy solo Linux tenga plataforma. Si esto falla, algo de
# un sistema se ha colado fuera de src/plataforma/.
#
# Luau se compila de sus fuentes en C++, y para otro sistema eso pide un
# compilador cruzado. Si no lo hay, ese sistema se comprueba sin la lógica en
# Luau —que es Rust sin nada de sistema— y se dice.
set -e
nativo=$(rustc -vV | sed -n 's/^host: //p')
comprobar() {
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
    if [ "$t" = "$nativo" ] || command -v "$cxx" >/dev/null 2>&1; then
        comprobar "$t" "" && echo "bien"
    else
        comprobar "$t" "--no-default-features" && echo "bien (sin Luau: aquí no hay $cxx)"
    fi
done
