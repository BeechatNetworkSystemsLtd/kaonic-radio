#!/bin/bash

set -euo pipefail

BASE_DIR="$(pwd -P)"

usage() {
    cat <<'EOF'
Usage: create-ota.sh [-b BUILD_DIR] [-o OUTPUT_DIR] [-s SIGN_KEY] [-k]

Create a Kaonic OTA package.

Options:
  -b, --build-dir   Path to build directory
  -o, --output-dir  Path to output directory
  -s, --sign-key    Path to PEM private key used for signing
  -k, --keep        Keep the unpacked OTA directory
  -h, --help        Show this help message
EOF
}

build_dir="${BASE_DIR}/target/armv7-unknown-linux-gnueabihf/release"
output_dir="${BASE_DIR}/deploy"
sign_key=""
keep=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        -b|--build-dir)
            build_dir="$2"
            shift 2
            ;;
        -o|--output-dir)
            output_dir="$2"
            shift 2
            ;;
        -s|--sign-key)
            sign_key="$2"
            shift 2
            ;;
        -k|--keep)
            keep=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown argument: $1" >&2
            usage >&2
            exit 1
            ;;
    esac
done

command -v openssl >/dev/null 2>&1 || {
    echo "openssl is required" >&2
    exit 1
}

command -v zip >/dev/null 2>&1 || {
    echo "zip is required" >&2
    exit 1
}

if [[ ! -d "$build_dir" ]]; then
    echo "Build directory not found: $build_dir" >&2
    exit 1
fi

mkdir -p "$output_dir"

build_dir="$(cd "$build_dir" && pwd -P)"
output_dir="$(cd "$output_dir" && pwd -P)"

binary_path="${build_dir}/kaonic-commd"
service_path="${BASE_DIR}/kaonic-commd/kaonic-commd.service"
plugin_toml_path="${BASE_DIR}/kaonic-commd/kaonic-plugin.toml"
release_name="kaonic-comm-ota"
release_path="${output_dir}/${release_name}"
release_archive_path="${output_dir}/${release_name}.zip"

if [[ ! -f "$binary_path" ]]; then
    echo "Binary not found: $binary_path" >&2
    exit 1
fi

if [[ ! -f "$service_path" ]]; then
    echo "Service file not found: $service_path" >&2
    exit 1
fi

if [[ ! -f "$plugin_toml_path" ]]; then
    echo "Plugin manifest not found: $plugin_toml_path" >&2
    exit 1
fi

if [[ -n "$sign_key" && ! -f "$sign_key" ]]; then
    echo "Signing key not found: $sign_key" >&2
    exit 1
fi

version="$(git describe --tags --long 2>/dev/null || true)"
if [[ -z "$version" ]]; then
    version="v0.0.0"
fi

tmp_dir=""
cleanup() {
    if [[ -n "$tmp_dir" && -d "$tmp_dir" ]]; then
        rm -rf "$tmp_dir"
    fi
}
trap cleanup EXIT

echo "Version: $version"
echo "> Prepare deploy directory"

rm -f "$release_archive_path"
rm -rf "$release_path"

echo "> Make directories"
mkdir -p "$release_path"

echo "> Copy files"
cp "$binary_path" "$release_path/kaonic-commd"
cp "$service_path" "$release_path/kaonic-commd.service"
cp "$plugin_toml_path" "$release_path/kaonic-plugin.toml"
printf '%s' "$version" > "$release_path/kaonic-commd.version"
openssl dgst -sha256 "$release_path/kaonic-commd" | awk '{print $NF}' > "$release_path/kaonic-commd.sha256"

if [[ -n "$sign_key" ]]; then
    tmp_dir="$(mktemp -d)"
    pub_key_path="${tmp_dir}/ota_sign_key.pub.pem"

    echo "> Sign $release_path/kaonic-commd"
    openssl dgst -sha256 -sign "$sign_key" -out "$release_path/kaonic-commd.sig" "$release_path/kaonic-commd"

    echo "> Verify $release_path/kaonic-commd"
    openssl pkey -in "$sign_key" -pubout -out "$pub_key_path" >/dev/null 2>&1
    openssl dgst -sha256 -verify "$pub_key_path" -signature "$release_path/kaonic-commd.sig" "$release_path/kaonic-commd" >/dev/null
fi

(cd "$release_path" && zip -q -r "$release_archive_path" .)

if [[ "$keep" -ne 1 ]]; then
    rm -rf "$release_path"
fi

echo "OTA Package: $release_archive_path"
