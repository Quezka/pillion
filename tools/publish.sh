#!/usr/bin/env bash
# Publishes Pillion for the first time: a private GitHub repo with the signing key as CI secrets, a backup on
# git.lan, and the v0.1.0 tag that starts the release build. Run it from anywhere, once. Safe to run again.
set -euo pipefail
cd "$(dirname "$0")/.."

GITHUB_REPO=Quezka/pillion
LAN_URL=git@git.lan:quezka/pillion.git
TAG="v$(grep -m1 -oP '^version = "\K[^"]+' Cargo.toml)"

if ! gh repo view "$GITHUB_REPO" >/dev/null 2>&1; then
    gh repo create "$GITHUB_REPO" --private \
        --description "A riding companion for an electric moped: battery, range, spoken directions and a voice mode for riding with an earpiece"
fi
git remote get-url origin >/dev/null 2>&1 || git remote add origin "git@github.com:$GITHUB_REPO.git"
git remote get-url lan >/dev/null 2>&1 || git remote add lan "$LAN_URL"

# The secrets go in before the tag, because the tag starts the build that needs them.
base64 -w0 android/pillion.jks | gh secret set PILLION_KEYSTORE_B64 -R "$GITHUB_REPO"
grep -oP '^storePassword=\K.*' android/keystore.properties | tr -d '\n' | gh secret set PILLION_KEYSTORE_PASSWORD -R "$GITHUB_REPO"

git push -u origin main
if git push lan main "$TAG"; then
    echo "Backed up to git.lan."
else
    echo "The git.lan backup didn't go through. If the server doesn't create repos on push, make an empty private" >&2
    echo "quezka/pillion in Gitea, then run this script again." >&2
fi
git push origin "$TAG"

echo
echo "Building. When it finishes, open this on your phone, signed in to GitHub, and download the APK:"
echo "https://github.com/$GITHUB_REPO/releases/latest"
