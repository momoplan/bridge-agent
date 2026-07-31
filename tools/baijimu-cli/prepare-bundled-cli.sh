#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
default_cli_git_url="${BAIJIMU_CLI_RS_GIT_URL:-https://gitee.com/zxflimit_admin/baijimu-cli-rs.git}"
cli_version_file="${repo_root}/tools/baijimu-cli/VERSION"
if [ ! -f "${cli_version_file}" ]; then
  echo "Missing pinned Baijimu CLI version: ${cli_version_file}" >&2
  exit 1
fi
pinned_cli_version="$(tr -d '[:space:]' < "${cli_version_file}")"
cli_git_ref="${BAIJIMU_CLI_RS_GIT_REF:-v${pinned_cli_version}}"
resource_dir="${BAIJIMU_CLI_RESOURCE_DIR:-${repo_root}/src-tauri/resources/bin}"

platform_name=""
binary_name="baijimu"
case "$(uname -s)" in
  Darwin)
    platform_name="macos-universal"
    ;;
  MINGW*|MSYS*|CYGWIN*|Windows_NT)
    platform_name="windows-x64"
    binary_name="baijimu.exe"
    ;;
  Linux)
    platform_name="linux-x64"
    ;;
  *)
    echo "Unsupported Baijimu CLI platform: $(uname -s)" >&2
    exit 1
    ;;
esac

if [ "${BAIJIMU_CLI_USE_RELEASE_ASSET:-false}" = "true" ]; then
  release_repo="${BAIJIMU_CLI_RELEASE_REPO:-momoplan/bridge-agent}"
  release_tag="baijimu-cli-v${pinned_cli_version}"
  asset_name="baijimu-cli-${pinned_cli_version}-${platform_name}.zip"
  checksum_file="${BAIJIMU_CLI_CHECKSUM_FILE:-${repo_root}/tools/baijimu-cli/SHA256SUMS}"
  if [ ! -f "${checksum_file}" ]; then
    echo "Missing pinned Baijimu CLI checksums: ${checksum_file}" >&2
    exit 1
  fi
  expected_sha256="$(
    awk -v asset="${asset_name}" '$2 == asset { print $1 }' "${checksum_file}"
  )"
  if [ -z "${expected_sha256}" ]; then
    echo "Missing pinned checksum for ${asset_name}: ${checksum_file}" >&2
    exit 1
  fi

  temporary_dir="$(mktemp -d)"
  trap 'rm -rf "${temporary_dir}"' EXIT
  if [ -n "${BAIJIMU_CLI_RELEASE_ASSETS_DIR:-}" ]; then
    cp \
      "${BAIJIMU_CLI_RELEASE_ASSETS_DIR}/${asset_name}" \
      "${temporary_dir}/${asset_name}"
  else
    if ! command -v gh >/dev/null 2>&1; then
      echo "GitHub CLI is required to download ${release_tag}" >&2
      exit 1
    fi
    gh release download "${release_tag}" \
      --repo "${release_repo}" \
      --dir "${temporary_dir}" \
      --pattern "${asset_name}"
  fi

  actual_sha256="$(
    if command -v sha256sum >/dev/null 2>&1; then
      sha256sum "${temporary_dir}/${asset_name}" | awk '{ print $1 }'
    else
      shasum -a 256 "${temporary_dir}/${asset_name}" | awk '{ print $1 }'
    fi
  )"
  if [ "${actual_sha256}" != "${expected_sha256}" ]; then
    echo "Baijimu CLI release checksum mismatch for ${asset_name}: expected ${expected_sha256}, got ${actual_sha256}" >&2
    exit 1
  fi

  extracted_dir="${temporary_dir}/extracted"
  mkdir -p "${extracted_dir}" "${resource_dir}"
  if [ "${platform_name}" = "windows-x64" ]; then
    BAIJIMU_CLI_ARCHIVE="${temporary_dir}/${asset_name}" \
      BAIJIMU_CLI_DESTINATION="${extracted_dir}" \
      powershell -NoProfile -Command '
        Expand-Archive `
          -Force `
          -LiteralPath $env:BAIJIMU_CLI_ARCHIVE `
          -DestinationPath $env:BAIJIMU_CLI_DESTINATION
      '
  else
    unzip -q "${temporary_dir}/${asset_name}" -d "${extracted_dir}"
  fi
  released_binary="${extracted_dir}/bin/${binary_name}"
  if [ ! -f "${released_binary}" ]; then
    echo "Baijimu CLI release asset does not contain bin/${binary_name}" >&2
    exit 1
  fi
  cp "${released_binary}" "${resource_dir}/${binary_name}"
  chmod 755 "${resource_dir}/${binary_name}" 2>/dev/null || true
  "${resource_dir}/${binary_name}" --version --json
  echo "Prepared pinned Baijimu CLI release asset ${asset_name} (${actual_sha256})"
  exit 0
fi

if [ -n "${BAIJIMU_CLI_RS_DIR:-}" ]; then
  cli_dir="${BAIJIMU_CLI_RS_DIR}"
elif [ -f "${repo_root}/../baijimu-cli-rs/Cargo.toml" ]; then
  cli_dir="${repo_root}/../baijimu-cli-rs"
else
  cli_dir="${repo_root}/../../baijimu-cli-rs"
fi

if [ ! -f "${cli_dir}/Cargo.toml" ]; then
  clone_dir="${RUNNER_TEMP:-/tmp}/baijimu-cli-rs"
  rm -rf "${clone_dir}"
  clone_url="${default_cli_git_url}"
  if [ -n "${BAIJIMU_CLI_RS_GIT_TOKEN:-}" ] && [[ "${clone_url}" == https://gitee.com/* ]]; then
    clone_url="https://oauth2:${BAIJIMU_CLI_RS_GIT_TOKEN}@${clone_url#https://}"
  fi
  clone_succeeded=false
  for attempt in 1 2 3 4 5; do
    rm -rf "${clone_dir}"
    if GIT_TERMINAL_PROMPT=0 git -c credential.helper= clone \
      --depth 1 \
      --branch "${cli_git_ref}" \
      "${clone_url}" \
      "${clone_dir}"; then
      clone_succeeded=true
      break
    fi
    if [ "${attempt}" -lt 5 ]; then
      retry_delay=$((attempt * 5))
      echo "Baijimu CLI clone attempt ${attempt}/5 failed; retrying in ${retry_delay}s" >&2
      sleep "${retry_delay}"
    fi
  done
  if [ "${clone_succeeded}" != "true" ]; then
    echo "Failed to clone Baijimu CLI ${cli_git_ref} after 5 attempts" >&2
    exit 1
  fi
  cli_dir="${clone_dir}"
fi

actual_cli_version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "${cli_dir}/Cargo.toml" | head -1)"
if [ "${actual_cli_version}" != "${pinned_cli_version}" ]; then
  echo "Baijimu CLI version mismatch: expected ${pinned_cli_version}, got ${actual_cli_version}" >&2
  exit 1
fi

mkdir -p "${resource_dir}"

if [ "$(uname -s)" = "Darwin" ]; then
  export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-10.13}"
  echo "Building bundled Baijimu CLI with MACOSX_DEPLOYMENT_TARGET=${MACOSX_DEPLOYMENT_TARGET}"
fi

if [ "$(uname -s)" = "Darwin" ] && [ "${BAIJIMU_CLI_RS_MACOS_UNIVERSAL:-}" = "true" ]; then
  cargo build --release --target x86_64-apple-darwin --manifest-path "${cli_dir}/Cargo.toml"
  cargo build --release --target aarch64-apple-darwin --manifest-path "${cli_dir}/Cargo.toml"
  lipo -create \
    "${cli_dir}/target/x86_64-apple-darwin/release/${binary_name}" \
    "${cli_dir}/target/aarch64-apple-darwin/release/${binary_name}" \
    -output "${resource_dir}/${binary_name}"
else
  cargo build --release --manifest-path "${cli_dir}/Cargo.toml"
  cp "${cli_dir}/target/release/${binary_name}" "${resource_dir}/${binary_name}"
fi

chmod 755 "${resource_dir}/${binary_name}" 2>/dev/null || true

"${resource_dir}/${binary_name}" --version --json
