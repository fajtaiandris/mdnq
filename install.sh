#!/bin/sh
set -eu

# Published release installer. Override MDNQ_REPO or MDNQ_INSTALL_DIR for forks.
repo=${MDNQ_REPO:-andrisfajtai/mdnq}
install_dir=${MDNQ_INSTALL_DIR:-"${HOME}/.local/bin"}
base="https://github.com/${repo}/releases/latest/download"

case "$(uname -s)" in
  Darwin) os=apple-darwin ;;
  Linux) os=unknown-linux-gnu ;;
  *) echo "mdnq: unsupported operating system" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  arm64|aarch64) arch=aarch64 ;;
  x86_64|amd64) arch=x86_64 ;;
  *) echo "mdnq: unsupported architecture" >&2; exit 1 ;;
esac

asset="mdnq-${arch}-${os}.tar.gz"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

curl -fLsS --retry 3 "${base}/${asset}" -o "${tmp}/${asset}"
curl -fLsS --retry 3 "${base}/SHA256SUMS" -o "${tmp}/SHA256SUMS"
expected=$(awk -v name="$asset" '$2 == name { print $1 }' "${tmp}/SHA256SUMS")
case "$expected" in
  *[!0-9a-fA-F]*|'') echo "mdnq: invalid release checksum" >&2; exit 1 ;;
esac
if [ "${#expected}" -ne 64 ]; then echo "mdnq: invalid release checksum" >&2; exit 1; fi
if command -v shasum >/dev/null 2>&1; then
  actual=$(shasum -a 256 "${tmp}/${asset}" | awk '{print $1}')
else
  actual=$(sha256sum "${tmp}/${asset}" | awk '{print $1}')
fi
if [ "$actual" != "$expected" ]; then echo "mdnq: checksum mismatch" >&2; exit 1; fi

tar -xzf "${tmp}/${asset}" -C "$tmp" mdnq
mkdir -p "$install_dir"
chmod 755 "${tmp}/mdnq"
mv "${tmp}/mdnq" "${install_dir}/mdnq"
echo "Installed mdnq to ${install_dir}/mdnq"
case ":${PATH}:" in
  *":${install_dir}:"*) ;;
  *) echo "Add ${install_dir} to your PATH, then run mdnq." ;;
esac
