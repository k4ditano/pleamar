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
        bien) echo "$salida" | grep -q ": ok ·" || { echo "✗ $f: tenía que estar bien"; echo "$salida" | head -3; mal=$((mal + 1)); } ;;
        fallo*) trozo=$(echo "$espera" | sed 's|^fallo «||; s|»$||')
            echo "$salida" | grep -qF "$trozo" || { echo "✗ $f: esperaba un fallo con «$trozo»"; echo "$salida" | head -3; mal=$((mal + 1)); } ;;
    esac
done

# El vocabulario de la referencia tiene que ser el que consulta el compilador, palabra por palabra.
awk '/^```vocabulario$/ { dentro = 1; next } /^```$/ { dentro = 0 } dentro' docs/11-referencia-del-lenguaje.md > "$ejemplos/escrito.txt"
./target/release/pleamar --gramatica > "$ejemplos/de-verdad.txt"
if ! diff -q "$ejemplos/escrito.txt" "$ejemplos/de-verdad.txt" > /dev/null; then
    echo "✗ la referencia (docs/11, §17) y el compilador no dicen lo mismo:"
    diff "$ejemplos/escrito.txt" "$ejemplos/de-verdad.txt" | sed 's/^</  la nota: /; s/^>/  pleamar: /' | grep -v "^[0-9-]"
    mal=$((mal + 1))
fi
n=$((n + 1))

# Cada sentencia tiene que poder explicarse en el editor: `--lsp` la enseña al pasar por encima.
sin_ayuda=""
explicadas=$(grep "^documented: " "$ejemplos/de-verdad.txt")
for palabra in $(grep "^statements: " "$ejemplos/de-verdad.txt" | cut -d' ' -f2-); do
    case " $explicadas " in *" $palabra "*) ;; *) sin_ayuda="$sin_ayuda $palabra" ;; esac
done
if [ -n "$sin_ayuda" ]; then
    echo "✗ sentencias que el editor no sabe explicar (voz::AYUDA):$sin_ayuda"
    mal=$((mal + 1))
fi
n=$((n + 1))

# Y el resaltado que se reparte en editor/ tiene que ser el que sale del vocabulario de ahora.
for par in "vim:editor/plm.vim" "vscode:editor/plm.tmLanguage.json"; do
    ./target/release/pleamar --resaltado "${par%%:*}" > "$ejemplos/resaltado.txt"
    if ! diff -q "$ejemplos/resaltado.txt" "${par#*:}" > /dev/null; then
        echo "✗ ${par#*:} se quedó atrás: hazlo otra vez con \`pleamar --resaltado ${par%%:*}\`"
        mal=$((mal + 1))
    fi
    n=$((n + 1))
done

# Y cada palabra del vocabulario tiene que salir en alguna prueba: una que nadie usa es una que nadie vigila.
cat pruebas/*.plm pruebas/comun/*.plm escenas/*.plm escenas/comun/*.plm "$ejemplos"/*.plm > "$ejemplos/todo.txt"
sin_uso=""
while IFS= read -r linea; do
    lista=${linea%%:*}
    case "$lista" in language|units|documented) continue ;; esac
    for palabra in ${linea#*: }; do
        grep -qw -- "$palabra" "$ejemplos/todo.txt" || sin_uso="$sin_uso $lista/$palabra"
    done
done < "$ejemplos/de-verdad.txt"
if [ -n "$sin_uso" ]; then
    echo "✗ palabras del vocabulario que ninguna prueba usa:$sin_uso"
    mal=$((mal + 1))
fi
n=$((n + 1))
rm -rf "$ejemplos"
[ "$mal" -eq 0 ] && echo "lenguaje · $n comprobaciones, todas como se esperaba" || { echo "lenguaje · $mal de $n no cumplen"; exit 1; }
