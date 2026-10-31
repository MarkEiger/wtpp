# Vendored dependencies

## i-slint-core 1.18.1

Patched in `graphics/wgpu_30.rs` (search for "wtpp patch"): when Slint creates
the wgpu instance for a window, it now passes the window's display handle as
`InstanceDescriptor::display`. Upstream passes `None`, which makes wgpu's
GLES backend use a surfaceless EGL display that can't present into a window.

This only matters for the OpenGL backend: WSLg (Mesa d3d12 → host GPU), and
Linux machines without a Vulkan driver. Vulkan, Metal and DX12 are unaffected.

Drop this once Slint ships the fix upstream.
