#!/bin/sh
# La guarda del enfoque multiplataforma: el núcleo tiene que compilar para los
# tres sistemas, aunque hoy solo Linux tenga plataforma. Si esto falla, algo de
# un sistema se ha colado fuera de src/plataforma/.
set -e
for t in x86_64-unknown-linux-gnu x86_64-pc-windows-gnu aarch64-apple-darwin; do
    printf '%-28s ' "$t"
    cargo check --quiet --target "$t" 2>&1 | grep -E "^error" | head -3 | grep . && exit 1 || echo "bien"
done
