#!/usr/bin/env bash
# Build the phone web app (dist-web/) and deploy it to Vercel production.
#   TC_WEB_DEPLOY_DIR=<folder linked to your Vercel project> scripts/deploy-web.sh
# The deploy folder lives outside the repo so its .vercel/ link survives rebuilds.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
dest="${TC_WEB_DEPLOY_DIR:?set TC_WEB_DEPLOY_DIR to the folder linked to your Vercel project}"
(cd "$root" && npm run -s build:web)
mkdir -p "$dest"
# Replace the previous build but keep the .vercel/ project link.
find "$dest" -mindepth 1 -maxdepth 1 ! -name .vercel -exec rm -r {} +
ditto "$root/dist-web" "$dest"
(cd "$dest" && vercel deploy --prod --yes ${VERCEL_SCOPE:+--scope "$VERCEL_SCOPE"})
