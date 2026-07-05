#!/usr/bin/env bash
set -euo pipefail

# Mirrors official OpenAI Codex release artifacts into a baijimu-controlled OSS
# prefix and publishes a SHA256 manifest for installer fallback use.

OSSUTIL="${OSSUTIL:-}"
if [ -z "$OSSUTIL" ]; then
  if command -v ossutil >/dev/null 2>&1; then
    OSSUTIL="$(command -v ossutil)"
  elif [ -x "$HOME/.aliyun/ossutil" ]; then
    OSSUTIL="$HOME/.aliyun/ossutil"
  else
    echo "ossutil not found; set OSSUTIL=/path/to/ossutil" >&2
    exit 2
  fi
fi

OSS_BUCKET="${OSS_BUCKET:-lowcode-common}"
OSS_PREFIX="${OSS_PREFIX:-codex-artifacts}"
OSS_REGION="${OSS_REGION:-cn-beijing}"
OSS_ENDPOINT="${OSS_ENDPOINT:-oss-cn-beijing.aliyuncs.com}"
OSS_CONFIG_FILE="${OSS_CONFIG_FILE:-$HOME/.ossutilconfig}"
OSS_PUBLIC_BASE_URL="${OSS_PUBLIC_BASE_URL:-https://${OSS_BUCKET}.${OSS_ENDPOINT}}"
GITHUB_RELEASE_API="${GITHUB_RELEASE_API:-https://api.github.com/repos/openai/codex/releases/latest}"
export CURL_EXTRA_ARGS="${CURL_EXTRA_ARGS:---http1.1}"
INCLUDE_OFFICIAL_APP_DMG="${INCLUDE_OFFICIAL_APP_DMG:-0}"
OFFICIAL_APP_DMG_ARCHES="${OFFICIAL_APP_DMG_ARCHES:-arm64 x86_64}"

# Keep the default set intentionally narrow enough for production installer use.
# Override CODEX_ASSET_REGEX to mirror additional official release assets.
CODEX_ASSET_REGEX="${CODEX_ASSET_REGEX:-^(install\\.sh|install\\.ps1|codex-package_SHA256SUMS|codex-(aarch64|x86_64)-apple-darwin\\.(tar\\.gz|dmg)|codex-(aarch64|x86_64)-pc-windows-msvc\\.exe\\.zip|codex-npm-(darwin-(arm64|x64)|win32-(arm64|x64)|linux-(arm64|x64)|[0-9].*)\\.tgz)$}"

WORK_DIR="${WORK_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/codex-artifacts.XXXXXX")}"
KEEP_WORK_DIR="${KEEP_WORK_DIR:-0}"

cleanup() {
  if [ "$KEEP_WORK_DIR" != "1" ]; then
    rm -rf "$WORK_DIR"
  fi
}
trap cleanup EXIT

mkdir -p "$WORK_DIR"

echo "Fetching release metadata from $GITHUB_RELEASE_API"
INCLUDE_OFFICIAL_APP_DMG="$INCLUDE_OFFICIAL_APP_DMG" OFFICIAL_APP_DMG_ARCHES="$OFFICIAL_APP_DMG_ARCHES" python3 - "$GITHUB_RELEASE_API" "$CODEX_ASSET_REGEX" "$WORK_DIR/release.json" "$WORK_DIR/assets.json" <<'PY'
import json
import os
import re
import sys
import urllib.request

api_url, pattern, release_path, assets_path = sys.argv[1:]
req = urllib.request.Request(
    api_url,
    headers={
        "Accept": "application/vnd.github+json",
        "User-Agent": "baijimu-codex-artifact-sync",
    },
)
with urllib.request.urlopen(req, timeout=60) as response:
    release = json.load(response)

regex = re.compile(pattern)
assets = [
    {
        "name": asset["name"],
        "size": asset.get("size"),
        "upstream_url": asset["browser_download_url"],
        "content_type": asset.get("content_type"),
    }
    for asset in release.get("assets", [])
    if regex.search(asset.get("name", ""))
]

if os.environ.get("INCLUDE_OFFICIAL_APP_DMG") == "1":
    app_assets = {
        "arm64": {
            "name": "codex-app-aarch64-apple-darwin.dmg",
            "size": None,
            "upstream_url": "https://persistent.oaistatic.com/codex-app-prod/Codex.dmg",
            "content_type": "application/x-apple-diskimage",
        },
        "x86_64": {
            "name": "codex-app-x86_64-apple-darwin.dmg",
            "size": None,
            "upstream_url": "https://persistent.oaistatic.com/codex-app-prod/Codex-latest-x64.dmg",
            "content_type": "application/x-apple-diskimage",
        },
    }
    for arch in os.environ.get("OFFICIAL_APP_DMG_ARCHES", "").split():
        if arch not in app_assets:
            raise SystemExit(f"unsupported OFFICIAL_APP_DMG_ARCHES entry: {arch}")
        assets.append(app_assets[arch])

if not assets:
    raise SystemExit(f"no release assets matched CODEX_ASSET_REGEX={pattern!r}")

release_summary = {
    "tag_name": release.get("tag_name"),
    "name": release.get("name"),
    "published_at": release.get("published_at"),
    "html_url": release.get("html_url"),
    "upstream_api_url": api_url,
}

with open(release_path, "w", encoding="utf-8") as f:
    json.dump(release_summary, f, ensure_ascii=False, indent=2)
    f.write("\n")

with open(assets_path, "w", encoding="utf-8") as f:
    json.dump(assets, f, ensure_ascii=False, indent=2)
    f.write("\n")

print(release_summary["tag_name"])
for asset in assets:
    print(asset["name"])
PY

TAG="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["tag_name"])' "$WORK_DIR/release.json")"
RELEASE_DIR="$WORK_DIR/releases/$TAG"
mkdir -p "$RELEASE_DIR"

python3 - "$WORK_DIR/assets.json" "$RELEASE_DIR" <<'PY'
import json
import os
import pathlib
import shlex
import subprocess
import sys

assets = json.load(open(sys.argv[1], encoding="utf-8"))
release_dir = pathlib.Path(sys.argv[2])
for asset in assets:
    out = release_dir / asset["name"]
    if out.exists() and out.stat().st_size == asset.get("size"):
        print(f"reuse {asset['name']}")
        continue
    print(f"download {asset['name']} <- {asset['upstream_url']}")
    command = [
        "curl",
        "-L",
        "--fail",
        "--retry",
        "8",
        "--retry-all-errors",
        "--retry-delay",
        "3",
        "--connect-timeout",
        "30",
        "--max-time",
        "1200",
    ]
    command.extend(shlex.split(os.environ.get("CURL_EXTRA_ARGS", "--http1.1")))
    command.extend(["-o", str(out), asset["upstream_url"]])
    subprocess.run(
        command,
        check=True,
    )
PY

python3 - "$WORK_DIR/release.json" "$WORK_DIR/assets.json" "$RELEASE_DIR" "$OSS_PUBLIC_BASE_URL" "$OSS_PREFIX" > "$RELEASE_DIR/manifest.json" <<'PY'
import datetime
import hashlib
import json
import pathlib
import sys

release_path, assets_path, release_dir, public_base, prefix = sys.argv[1:]
release = json.load(open(release_path, encoding="utf-8"))
assets = json.load(open(assets_path, encoding="utf-8"))
release_dir = pathlib.Path(release_dir)
fetched_at = datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0).isoformat()

manifest_assets = []
for asset in assets:
    path = release_dir / asset["name"]
    data = path.read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    if asset.get("size") is not None and len(data) != int(asset["size"]):
        raise SystemExit(f"size mismatch for {asset['name']}: expected {asset['size']} got {len(data)}")
    object_key = f"{prefix}/releases/{release['tag_name']}/{asset['name']}"
    manifest_assets.append(
        {
            "name": asset["name"],
            "upstream_url": asset["upstream_url"],
            "mirror_url": f"{public_base.rstrip('/')}/{object_key}",
            "object_key": object_key,
            "sha256": digest,
            "size": len(data),
            "content_type": asset.get("content_type"),
            "license_notice": "Mirrored from the official OpenAI Codex GitHub release without modification.",
        }
    )

json.dump(
    {
        "schema_version": 1,
        "source": "github.com/openai/codex",
        "upstream_release": release,
        "fetched_at": fetched_at,
        "assets": manifest_assets,
    },
    sys.stdout,
    ensure_ascii=False,
    indent=2,
)
print()
PY

cp "$RELEASE_DIR/manifest.json" "$WORK_DIR/latest.json"

OSS_FLAGS=(--config-file "$OSS_CONFIG_FILE" --region "$OSS_REGION" --endpoint "$OSS_ENDPOINT" --force --no-progress)

echo "Uploading release assets to oss://${OSS_BUCKET}/${OSS_PREFIX}/releases/${TAG}/"
"$OSSUTIL" cp "$RELEASE_DIR/" "oss://${OSS_BUCKET}/${OSS_PREFIX}/releases/${TAG}/" -r "${OSS_FLAGS[@]}"

echo "Publishing latest manifest to oss://${OSS_BUCKET}/${OSS_PREFIX}/latest.json"
"$OSSUTIL" cp "$WORK_DIR/latest.json" "oss://${OSS_BUCKET}/${OSS_PREFIX}/latest.json" \
  --content-type application/json \
  --cache-control "no-cache" \
  "${OSS_FLAGS[@]}"

echo "Published:"
echo "${OSS_PUBLIC_BASE_URL%/}/${OSS_PREFIX}/latest.json"
