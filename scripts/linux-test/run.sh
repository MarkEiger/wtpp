#!/usr/bin/env bash
# Run the Linux UI scenarios (on the test machine). ./run.sh [NAME...]
cd "$(dirname "$0")"
rm -rf "$HOME/wtpp-test"
# A private X display for the tests (no window manager: xdotool focuses
# windows directly).
export WTPP_TEST_DISPLAY=:99
Xvfb :99 -screen 0 1600x1000x24 -nolisten tcp >/dev/null 2>&1 &
xvfb=$!
trap 'kill $xvfb' EXIT
sleep 1
while IFS= read -r line; do
    [[ -z "$line" || "$line" == \#* ]] && continue
    name="${line%%: *}"; steps="${line#*: }"
    if [[ $# -gt 0 ]] && [[ ! " $* " == *" $name "* ]]; then continue; fi
    echo "== $name"
    ./drive.sh "$name" "$steps"
done < scenarios.txt
