#!/usr/bin/env bash
# Build release packages into dist/:
#   wtpp-<version>-windows-x64.zip   wtpp.exe + install.ps1 (native Windows)
#   wtpp-<version>-linux-x64.tar.gz  wtpp binary
set -euo pipefail

cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
rm -rf dist && mkdir -p dist

cargo build --release --target x86_64-pc-windows-gnu
win="dist/wtpp-$version-windows-x64"
mkdir -p "$win"
cp target/x86_64-pc-windows-gnu/release/wtpp.exe assets/wtpp.ico scripts/windows/install.ps1 README.md "$win/"
cp assets/fonts/LICENSE-Cascadia.txt "$win/"
(cd dist && python3 -m zipfile -c "$(basename "$win").zip" "$(basename "$win")" && rm -r "$(basename "$win")")

cargo build --release
linux="dist/wtpp-$version-linux-x64"
mkdir -p "$linux"
cp target/release/wtpp README.md assets/fonts/LICENSE-Cascadia.txt scripts/linux/install.sh "$linux/"
cp assets/wtpp.png "$linux/wtpp.png"
tar -C dist -czf "$linux.tar.gz" "$(basename "$linux")" && rm -r "$linux"

(cd dist && sha256sum ./*.zip ./*.tar.gz > SHA256SUMS)
ls -la dist
