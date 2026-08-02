#!/usr/bin/env sh
set -eu

PROJECT_ROOT=$(CDPATH= cd "$(dirname "$0")/.." && pwd)

# npm writes its cache and error logs below the project so verification never touches ~/.npm.
exec env npm_config_cache="$PROJECT_ROOT/.npm-cache" npm "$@"
