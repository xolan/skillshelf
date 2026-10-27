#!/usr/bin/env bash
# Writes <dist>/PKGBUILD from assets/arch/PKGBUILD, with the version and the checksums of the
# Linux tarballs in <dist>. Run it after the x86_64 and aarch64 tarballs are built.
set -euo pipefail
version="$1"
dist="$2"
root="$(cd "$(dirname "$0")/.." && pwd)"
sum() {
  local file="$dist/skillshelf-$version-$1-unknown-linux-gnu.tar.gz.sha256"
  local hash
  hash="$(cut -d' ' -f1 "$file")"
  [[ "$hash" =~ ^[0-9a-f]{64}$ ]] || { echo "no checksum in $file" >&2; exit 1; }
  echo "$hash"
}
x86_64="$(sum x86_64)"
aarch64="$(sum aarch64)"
sed -e "s/@VERSION@/$version/" \
  -e "s/@SHA256_X86_64@/$x86_64/" \
  -e "s/@SHA256_AARCH64@/$aarch64/" \
  "$root/assets/arch/PKGBUILD" > "$dist/PKGBUILD"
(cd "$dist" && sha256sum PKGBUILD > PKGBUILD.sha256)
echo "$dist/PKGBUILD"
