#!/usr/bin/env bash
# Drive wt++ for UI tests on a Linux machine (e.g. over ssh). wt++ runs on a
# private virtual X display (Xvfb, started by run.sh), so the tests never
# touch the user's desktop; xdotool sends it real X11 input.
#
#   drive.sh NAME 'STEP|STEP|...'
#
# Steps (coordinates are relative to the first wt++ window unless noted):
#   sleep:MS            keys:CHORD[ CHORD...]  (xdotool names: ctrl+shift+t)
#   type:TEXT           shot:NAME               (every wt++ window)
#   click:X,Y           drag:X1,Y1,X2,Y2        sdrag:X1,Y1,X2,Y2 (screen)
#   hdrag:X,Y           (drag from where the pointer is now)
#   wheel:X,Y,CLICKS    (mouse wheel at X,Y; positive = up)
#   windows             launch (start wt++ again: should open a window here)
#   clip                (print the clipboard)   size:W,H
#   plusnew             (click + in the other window, e.g. a torn-off one)
set -uo pipefail

name="$1"; steps="$2"
out="${WTPP_TEST_OUT:-$HOME/wtpp-test}"
exe="${WTPP_EXE:-$HOME/wtpp/target/release/wtpp}"
mkdir -p "$out"
export DISPLAY="${WTPP_TEST_DISPLAY:-:99}"
# The user's settings bus, for features that write desktop settings
# (default terminal).
export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$(id -u)/bus"
unset WAYLAND_DISPLAY XAUTHORITY
export WTPP_INSTANCE=test WTPP_LOG_FILE="$out/wtpp.log"

"$exe" >/dev/null 2>&1 &
pid=$!
for _ in $(seq 100); do
    win=$(xdotool search --pid "$pid" --onlyvisible 2>/dev/null | head -1)
    [ -n "$win" ] && break
    sleep 0.1
done
[ -z "${win:-}" ] && { echo "   no window"; kill $pid; exit 1; }
sleep 1.5
xdotool windowmove "$win" 60 60 windowsize "$win" 1100 640 windowfocus --sync "$win"
sleep 0.5

origin() { eval "$(xdotool getwindowgeometry --shell "$win")"; echo "$X $Y"; }
wins() { xdotool search --pid "$pid" --onlyvisible 2>/dev/null; }
pointer_drag() {  # x1 y1 x2 y2 in screen coordinates
    xdotool mousemove "$1" "$2" mousedown 1
    for i in $(seq 1 25); do
        xdotool mousemove $(( $1 + ($3 - $1) * i / 25 )) $(( $2 + ($4 - $2) * i / 25 )); sleep 0.02
    done
    sleep 0.3; xdotool mouseup 1; sleep 1
}

IFS='|' read -ra list <<< "$steps"
for s in "${list[@]}"; do
    # Keys only while wt++ has the focus.
    case "$s" in keys:*|type:*)
        focused=$(xdotool getwindowfocus 2>/dev/null)
        if ! wins | grep -qx "$focused"; then echo "   ABORTED at '$s': wt++ isn't focused"; break; fi;;
    esac
    case "$s" in
        sleep:*) sleep "$(echo "scale=3; ${s#sleep:}/1000" | bc)";;
        keys:*) xdotool key --delay 60 ${s#keys:}; sleep 0.5;;
        type:*) xdotool type --delay 30 "${s#type:}"; sleep 0.3;;
        click:*) IFS=, read -r x y <<< "${s#click:}"; read -r ox oy <<< "$(origin)"
                 xdotool mousemove $((ox + x)) $((oy + y)) click 1; sleep 0.6;;
        wheel:*) IFS=, read -r x y n <<< "${s#wheel:}"; read -r ox oy <<< "$(origin)"
                 button=4; if [ "$n" -lt 0 ]; then button=5; n=$((-n)); fi
                 xdotool mousemove $((ox + x)) $((oy + y)) click --repeat "$n" --delay 30 $button; sleep 0.5;;
        drag:*) IFS=, read -r x1 y1 x2 y2 <<< "${s#drag:}"; read -r ox oy <<< "$(origin)"
                pointer_drag $((ox + x1)) $((oy + y1)) $((ox + x2)) $((oy + y2));;
        sdrag:*) IFS=, read -r x1 y1 x2 y2 <<< "${s#sdrag:}"; pointer_drag "$x1" "$y1" "$x2" "$y2";;
        hdrag:*) IFS=, read -r x2 y2 <<< "${s#hdrag:}"; read -r ox oy <<< "$(origin)"
                 eval "$(xdotool getmouselocation --shell)"; pointer_drag "$X" "$Y" $((ox + x2)) $((oy + y2));;
        size:*) IFS=, read -r w h <<< "${s#size:}"; xdotool windowsize "$win" "$w" "$h"; sleep 0.6;;
        shot:*) k=0; for w in $(wins); do import -window "$w" "$out/$name-${s#shot:}-$k.png" 2>/dev/null; k=$((k + 1)); done;;
        windows) echo "   windows $(wins | wc -l)";;
        plusnew) other=$(wins | grep -vx "$win" | head -1)
                 eval "$(xdotool getwindowgeometry --shell "$other")"
                 xdotool mousemove $((X + 246)) $((Y + 22)) click 1; sleep 0.6;;
        focus:*) xdotool windowfocus --sync "$(wins | sed -n "$(( ${s#focus:} + 1 ))p")"; sleep 0.3;;
        launch) "$exe" >/dev/null 2>&1 & q=$!; sleep 2
                if kill -0 $q 2>/dev/null; then echo "   second launch still running"; kill $q; else echo "   second launch exited"; fi;;
        clip) echo "   clipboard: $(xclip -o -selection clipboard 2>/dev/null || xsel -b 2>/dev/null | head -c 80)";;
        *) echo "   unknown step $s";;
    esac
done
kill "$pid" 2>/dev/null
wait "$pid" 2>/dev/null
