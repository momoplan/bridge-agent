#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
default_cli_git_url="${BAIJIMU_CLI_RS_GIT_URL:-https://gitee.com/zxflimit_admin/baijimu-cli-rs.git}"
if [ -n "${BAIJIMU_CLI_RS_DIR:-}" ]; then
  cli_dir="${BAIJIMU_CLI_RS_DIR}"
elif [ -f "${repo_root}/../baijimu-cli-rs/Cargo.toml" ]; then
  cli_dir="${repo_root}/../baijimu-cli-rs"
else
  cli_dir="${repo_root}/../../baijimu-cli-rs"
fi
resource_dir="${repo_root}/src-tauri/resources/bin"

if [ ! -f "${cli_dir}/Cargo.toml" ]; then
  clone_dir="${RUNNER_TEMP:-/tmp}/baijimu-cli-rs"
  rm -rf "${clone_dir}"
  clone_url="${default_cli_git_url}"
  if [ -n "${BAIJIMU_CLI_RS_GIT_TOKEN:-}" ] && [[ "${clone_url}" == https://gitee.com/* ]]; then
    clone_url="${clone_url/https:\\/\\/gitee.com\\//https://oauth2:${BAIJIMU_CLI_RS_GIT_TOKEN}@gitee.com/}"
  fi
  git clone --depth 1 "${clone_url}" "${clone_dir}"
  cli_dir="${clone_dir}"
fi

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*|Windows_NT)
    binary_name="baijimu.exe"
    ;;
  *)
    binary_name="baijimu"
    ;;
esac

cargo build --release --manifest-path "${cli_dir}/Cargo.toml"
mkdir -p "${resource_dir}"
cp "${cli_dir}/target/release/${binary_name}" "${resource_dir}/${binary_name}"
chmod 755 "${resource_dir}/${binary_name}" 2>/dev/null || true

"${resource_dir}/${binary_name}" --version --json
