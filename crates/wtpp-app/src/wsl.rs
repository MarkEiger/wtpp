//! GPU setup when running inside WSL2 (WSLg).
//!
//! WSL's Mesa usually has no GPU Vulkan driver, but its OpenGL d3d12 driver
//! forwards to the Windows GPU driver. That driver needs `/usr/lib/wsl/lib` on
//! the library path, which the dynamic loader only reads at process start, so
//! we set the environment and re-exec ourselves once.

#[cfg(target_os = "linux")]
pub fn ensure_gpu_env() {
    use std::os::unix::process::CommandExt;

    const WSL_LIB: &str = "/usr/lib/wsl/lib";
    const MARKER: &str = "WTPP_WSL_GPU_ENV";

    let is_wsl = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .is_ok_and(|r| r.to_ascii_lowercase().contains("microsoft"));
    // Respect an explicit backend choice, and never loop.
    if !is_wsl || std::env::var_os(MARKER).is_some() || std::env::var_os("WGPU_BACKEND").is_some() {
        return;
    }
    if !std::path::Path::new(WSL_LIB).join("libd3d12.so").exists() {
        return;
    }

    let library_path = match std::env::var("LD_LIBRARY_PATH") {
        Ok(existing) if !existing.is_empty() => format!("{WSL_LIB}:{existing}"),
        _ => WSL_LIB.to_string(),
    };
    let Ok(exe) = std::env::current_exe() else { return };
    let err = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env(MARKER, "1")
        .env("LD_LIBRARY_PATH", library_path)
        .env("GALLIUM_DRIVER", "d3d12")
        .env("WGPU_BACKEND", "gl")
        .exec();
    log::warn!("could not re-exec with WSL GPU environment: {err}");
}

#[cfg(not(target_os = "linux"))]
pub fn ensure_gpu_env() {}
