#!/usr/bin/env bash
set -euo pipefail

: "${YAP_ARCH_VERSION:?Missing YAP_ARCH_VERSION}"
if [[ ! "$YAP_ARCH_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "Invalid package version: $YAP_ARCH_VERSION" >&2
  exit 1
fi

work_dir=$(mktemp -d)
useradd --create-home builder
install -m644 /input/yap.deb /source/LICENSE /source/packaging/arch/PKGBUILD "$work_dir/"
chown -R builder:builder "$work_dir"
cd "$work_dir"
runuser -u builder -- env YAP_ARCH_VERSION="$YAP_ARCH_VERSION" makepkg --nodeps

package="yet-another-player-${YAP_ARCH_VERSION//-/_}-1-x86_64.pkg.tar.zst"
pacman -Qip -- "$package"
install -m644 "$package" /output/
