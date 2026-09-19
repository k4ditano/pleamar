#!/bin/sh
# Pasa cada escena de pruebas/ por `--comprobar` y mira que ocurra lo que su primera línea espera.
cd "$(dirname "$0")" || exit 1
cargo build --release --quiet || exit 1
# Los ejemplos completos de la referencia (los bloques ```plm) también se comprueban:
# si la nota miente, esto falla.
ejemplos=$(mktemp -d)
awk -v dir="$ejemplos" '/^```plm$/ { k++; dentro = 1; next } /^```$/ { dentro = 0 } dentro { print > (dir "/referencia-" k ".plm") }' docs/11-referencia-del-lenguaje.md
mal=0; n=0
for f in pruebas/*.plm escenas/*.plm "$ejemplos"/*.plm; do
    n=$((n + 1))
    espera=$(head -1 "$f" | sed -n 's|^// espera: ||p')
    [ -z "$espera" ] && espera="bien"
    salida=$(./target/release/pleamar --comprobar "$f" 2>&1)
    case "$espera" in
        bien) echo "$salida" | grep -q ": bien ·" || { echo "✗ $f: tenía que estar bien"; echo "$salida" | head -3; mal=$((mal + 1)); } ;;
        fallo*) trozo=$(echo "$espera" | sed 's|^fallo «||; s|»$||')
            echo "$salida" | grep -qF "$trozo" || { echo "✗ $f: esperaba un fallo con «$trozo»"; echo "$salida" | head -3; mal=$((mal + 1)); } ;;
    esac
done
rm -rf "$ejemplos"
[ "$mal" -eq 0 ] && echo "lenguaje · $n escenas, todas como se esperaba" || { echo "lenguaje · $mal de $n no cumplen"; exit 1; }
