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
    # An aborted scenario (another app took the foreground) doesn't stop the run.
    (cd /mnt/c && powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$win_script" -StepList "$steps" -ArgList "$args" < /dev/null 2>&1 | tr -d '\r' | sed 's/^/   /') || true
done < scenarios.txt

# 2x2 contact sheets for review: sheet-N.png next to the screenshots.
(cd /mnt/c && powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$montage" < /dev/null 2>&1 | tr -d '\r')
