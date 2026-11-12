# Vendored dependencies

## i-slint-core 1.18.1

Patched in `graphics/wgpu_30.rs` (search for "wtpp patch"): when Slint creates
the wgpu instance for a window, it now passes the window's display handle as
`InstanceDescriptor::display`. Upstream passes `None`, which makes wgpu's
GLES backend use a surfaceless EGL display that can't present into a window.

This only matters for the OpenGL backend: WSLg (Mesa d3d12 → host GPU), and
Linux machines without a Vulkan driver. Vulkan, Metal and DX12 are unaffected.

Drop this once Slint ships the fix upstream.

## alacritty_terminal 0.26.0

Patched in `src/event_loop.rs` (search for "wtpp patch"): on Windows, the
PTY reader thread's readiness signal can get lost, which stalled all output
until some other event such as a keypress (`seq 1 20000` in WSL froze around
line 12,000). While output is flowing, the event loop now wakes every 4 ms
and reads anyway; when the shell is idle it waits as before.

Also in `src/event_loop.rs`: when the program exits, its output is read until
it has been quiet for 100 ms (2 s at most), not once, and the exit is
reported after that. ConPTY paints a program's last output on its own
schedule, so a program that printed and exited at once lost the end of it.
