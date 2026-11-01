#!/usr/bin/env bash
# Run Windows UI test scenarios against the installed wt++.
#   ./run.sh                 run every scenario in scenarios.txt
#   ./run.sh tabs find       run the named ones
# Screenshots: /mnt/c/Users/<you>/AppData/Local/Temp/wtpp-test/<scenario>-<shot>.png
set -euo pipefail
cd "$(dirname "$0")"
win_temp="$(wslpath "$(cmd.exe /c 'echo %TEMP%' 2>/dev/null | tr -d '\r')")/wtpp-test"
mkdir -p "$win_temp"
cp drive.ps1 montage.ps1 "$win_temp/"
win_script="$(wslpath -w "$win_temp/drive.ps1")"
montage="$(wslpath -w "$win_temp/montage.ps1")"
(cd /mnt/c && powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$montage" -Clean < /dev/null)

# wt++ remembers the last window's size and position in state.json; the
# tests close windows of every size. Keep the user's, and start every
# scenario from it.
state="$(wslpath "$(cmd.exe /c 'echo %LOCALAPPDATA%' 2>/dev/null | tr -d '\r')")/wtpp/state.json"
saved_state="$(mktemp)"
have_state=0
[[ -f "$state" ]] && cp "$state" "$saved_state" && have_state=1
put_back_state() { if [[ $have_state == 1 ]]; then cp "$saved_state" "$state"; else rm -f "$state"; fi; }
# The default-terminal scenarios change (and restore) the user's choice of
# default terminal: put back exactly what was there.
reg_dir="$(cmd.exe /c 'echo %TEMP%' 2>/dev/null | tr -d '\r')\\wtpp-test"
for key in 'HKCU\Console\%%Startup' 'HKCU\Software\Classes\CLSID\{1B6AF1D5-8948-4A5C-B2BB-7F7BF27CBA6A}' 'HKCU\Software\wtpp\DefaultTerminal'; do
    n=$((${n:-0} + 1))
    reg_keys+=("$key")
    if (cd /mnt/c && reg.exe query "$key" >/dev/null 2>&1); then
        (cd /mnt/c && reg.exe export "$key" "$reg_dir\\saved-$n.reg" /y >/dev/null 2>&1) || true
        # Present but not saved: leave it alone rather than lose it.
        [[ -f "$win_temp/saved-$n.reg" ]] && reg_state+=(saved) || reg_state+=(keep)
    else
        reg_state+=(absent)
    fi
done
put_back_registry() {
    local i
    for i in "${!reg_keys[@]}"; do
        local key="${reg_keys[$i]}" file="saved-$((i + 1)).reg"
        case "${reg_state[$i]}" in
            saved) (cd /mnt/c && reg.exe delete "$key" /f >/dev/null 2>&1 && reg.exe import "$reg_dir\\$file" >/dev/null 2>&1) || true
                   rm -f "$win_temp/$file";;
            absent) (cd /mnt/c && reg.exe delete "$key" /f >/dev/null 2>&1) || true;;
        esac
    done
}
trap 'put_back_state; rm -f "$saved_state"; put_back_registry' EXIT

while IFS= read -r line; do
    [[ -z "$line" || "$line" == \#* ]] && continue
    # "name: steps" or "name [wtpp args]: steps"
    head="${line%%: *}"
    steps="${line#*: }"
    name="${head%% *}"
    args=""
    if [[ "$head" == *"["*"]"* ]]; then args="${head#*[}"; args="${args%]}"; fi
    if [[ $# -gt 0 ]] && [[ ! " $* " == *" $name "* ]]; then continue; fi
    # Prefix screenshot names with the scenario name.
    steps="${steps//shot:/shot:$name-}"
    steps="${steps//shotall:/shotall:$name-}"
    steps="${steps//shotrect:/shotrect:$name-}"
    steps="${steps//others:/others:$name-}"
    echo "== $name"
    put_back_state
    # An aborted scenario (another app took the foreground) doesn't stop the run.
    (cd /mnt/c && powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$win_script" -StepList "$steps" -ArgList "$args" < /dev/null 2>&1 | tr -d '\r' | sed 's/^/   /') || true
done < scenarios.txt

# 2x2 contact sheets for review: sheet-N.png next to the screenshots.
(cd /mnt/c && powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$montage" < /dev/null 2>&1 | tr -d '\r')
