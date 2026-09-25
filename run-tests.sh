#!/bin/sh
# Runs every scene in tests/ through `--check` and checks that what its first line expects happens.
cd "$(dirname "$0")" || exit 1
cargo build --release --quiet || exit 1
# The complete examples in the documentation (the ```plm blocks) are checked too:
# if the README, the guide, the recipes or the reference lie, this fails.
examples=$(mktemp -d)
for note in README.md docs/11-language-reference.md docs/guide.md docs/recipes.md; do
    short=$(basename "$note" .md)
    awk -v dir="$examples" -v from="$short" '/^```plm$/ { k++; inside = 1; next } /^```$/ { inside = 0 } inside { print > (dir "/" from "-" k ".plm") }' "$note"
done
bad=0; n=0
for f in tests/*.plm examples/*.plm "$examples"/*.plm; do
    n=$((n + 1))
    expect=$(head -1 "$f" | sed -n 's|^// expect: ||p')
    [ -z "$expect" ] && expect="ok"
    output=$(./target/release/pleamar --check "$f" 2>&1)
    case "$expect" in
        ok) echo "$output" | grep -q ": ok ·" || { echo "✗ $f: it had to be fine"; echo "$output" | head -3; bad=$((bad + 1)); } ;;
        error*) piece=$(echo "$expect" | sed 's|^error «||; s|»$||')
            echo "$output" | grep -qF "$piece" || { echo "✗ $f: expected an error with «$piece»"; echo "$output" | head -3; bad=$((bad + 1)); } ;;
    esac
done

# The vocabulary in the reference has to be the one the compiler consults, word for word.
awk '/^```vocabulary$/ { inside = 1; next } /^```$/ { inside = 0 } inside' docs/11-language-reference.md > "$examples/written.txt"
./target/release/pleamar --grammar > "$examples/actual.txt"
if ! diff -q "$examples/written.txt" "$examples/actual.txt" > /dev/null; then
    echo "✗ the reference (docs/11, §17) and the compiler do not say the same:"
    diff "$examples/written.txt" "$examples/actual.txt" | sed 's/^</  the note: /; s/^>/  pleamar:  /' | grep -v "^[0-9-]"
    bad=$((bad + 1))
fi
n=$((n + 1))

# Every statement has to be explainable in the editor: `--lsp` shows it on hover.
unexplained=""
explained=$(grep "^documented: " "$examples/actual.txt")
for word in $(grep "^statements: " "$examples/actual.txt" | cut -d' ' -f2-); do
    case " $explained " in *" $word "*) ;; *) unexplained="$unexplained $word" ;; esac
done
if [ -n "$unexplained" ]; then
    echo "✗ statements the editor cannot explain (vocabulary::HELP):$unexplained"
    bad=$((bad + 1))
fi
n=$((n + 1))

# And the highlighting shipped in editor/ has to be the one today's vocabulary produces.
for pair in "vim:editor/plm.vim" "vscode:editor/plm.tmLanguage.json"; do
    ./target/release/pleamar --highlight "${pair%%:*}" > "$examples/highlight.txt"
    if ! diff -q "$examples/highlight.txt" "${pair#*:}" > /dev/null; then
        echo "✗ ${pair#*:} fell behind: make it again with \`pleamar --highlight ${pair%%:*}\`"
        bad=$((bad + 1))
    fi
    n=$((n + 1))
done

# And every word of the vocabulary has to appear in some test: one nobody uses is one nobody watches.
cat tests/*.plm tests/common/*.plm examples/*.plm examples/common/*.plm "$examples"/*.plm > "$examples/all.txt"
unused=""
while IFS= read -r line; do
    list=${line%%:*}
    case "$list" in language|units|documented) continue ;; esac
    for word in ${line#*: }; do
        grep -qw -- "$word" "$examples/all.txt" || unused="$unused $list/$word"
    done
done < "$examples/actual.txt"
if [ -n "$unused" ]; then
    echo "✗ vocabulary words no test uses:$unused"
    bad=$((bad + 1))
fi
n=$((n + 1))
rm -rf "$examples"
[ "$bad" -eq 0 ] && echo "language · $n checks, all as expected" || { echo "language · $bad of $n fail"; exit 1; }
