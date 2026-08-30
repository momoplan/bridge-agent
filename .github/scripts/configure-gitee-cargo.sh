#!/usr/bin/env bash
set -euo pipefail

: "${GITEE_ACCESS_TOKEN:?GITEE_ACCESS_TOKEN is required}"

git config --global credential.https://gitee.com.username oauth2
git config --global credential.https://gitee.com.helper \
  '!f() { test "$1" = get || exit 0; printf "%s\n" "username=oauth2" "password=$GITEE_ACCESS_TOKEN"; }; f'
