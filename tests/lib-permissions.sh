#!/bin/sh
# Language 0.11: an installed library with logic is a plugin like any other. Its `.luau` runs, but
# under `--layout` or `--no-permissions` it has no permissions at all: here it tries to run `touch`
# and the file must not appear (and it does appear once approved, as a control).
# Usage: tests/lib-permissions.sh path/to/pleamar   (no graphics card needed: the logic runs anyway)
set -u
B=${1:?the pleamar binary}
D=$(mktemp -d)
trap '[ "${KEEP:-}" ] || rm -rf "$D"' EXIT; [ "${KEEP:-}" ] && echo "$D"
M="$D/mark"
mkdir -p "$D/app/lib/tide" "$D/home"
printf 'name tide\nversion 1.0.0\nlanguage 0.11\n' > "$D/app/lib/tide/pleamar-lib"
printf 'library Spy {\n    permissions { run: "touch" }\n    component Spy() {\n        box { from: 0, 0; size: 4, 4; color: #ff0000 }\n    }\n}\n' > "$D/app/lib/tide/spy.plm"
printf 'run("touch", { "%s" }, function(out, code) end)\n' "$M" > "$D/app/lib/tide/spy.luau"
printf 'import "lib:tide/spy"\nscene App {\n    surface { size: 40, 40; anchor: center }\n    Spy()\n}\n' > "$D/app/app.plm"
export HOME="$D/home" XDG_CONFIG_HOME="$D/home/.config" XDG_DATA_HOME="$D/home/.local/share" XDG_CACHE_HOME="$D/home/.cache" XDG_STATE_HOME="$D/home/.local/state" PLEAMAR_NO_SKILL_REFRESH=1
fail=0
"$B" --check "$D/app/app.plm" >/dev/null 2>&1 || { echo "FAIL: --check"; fail=1; }
"$B" --approve "$D/app/app.plm" --yes >/dev/null 2>&1
# Runs a case until its logic has really started (on a machine without a graphics card the
# process can end before the logic starts): what it says is then judged. 1 = it ran, 0 = never.
case_runs() {
  for _ in 1 2 3 4 5; do
    rm -f "$M"
    timeout 60 "$B" "$@" >"$D/log" 2>&1
    grep -q "spy.luau" "$D/log" && return 0
  done
  return 1
}
case_runs --layout "$D/app/app.plm" || { echo "FAIL: --layout never started the library's logic"; fail=1; }
[ -e "$M" ] && { echo "FAIL: --layout let the library's logic run a program"; fail=1; }
grep -q "has no permission to run 'touch'" "$D/log" || { echo "FAIL: under --layout the library's logic was not refused"; fail=1; }
case_runs --no-permissions --scene "$D/app/app.plm" --film "$D/film" --seconds 1 || { echo "FAIL: --no-permissions never started the library's logic"; fail=1; }
[ -e "$M" ] && { echo "FAIL: --no-permissions let the library's logic run a program"; fail=1; }
grep -q "has no permission to run 'touch'" "$D/log" || { echo "FAIL: under --no-permissions the library's logic was not refused"; fail=1; }
# Control: approved, without those flags, it does run it.
ok=0
for _ in 1 2 3 4 5; do
  rm -f "$M"
  timeout 60 "$B" --scene "$D/app/app.plm" --film "$D/film2" --seconds 1 >"$D/log" 2>&1
  [ -e "$M" ] && { ok=1; break; }
done
[ $ok = 1 ] || { echo "FAIL: control: approved, the library's logic should have run 'touch'"; fail=1; }
[ $fail = 0 ] && echo "ok: a library's logic runs with no permissions under --layout and --no-permissions"
exit $fail
