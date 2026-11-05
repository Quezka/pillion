#!/usr/bin/env bash
# Publishes Pillion: a backup of the code and of the signing key on git.lan, a private GitHub repo with the key as CI
# secrets, and the tag that starts the release build. Run it from anywhere. Safe to run again.
set -euo pipefail
cd "$(dirname "$0")/.."

GITHUB_REPO=Quezka/pillion
LAN_HOST=git.lan
LAN_CODE=quezka/pillion
LAN_SIGNING=quezka/pillion-signing
TAG="v$(grep -m1 -oP '^version = "\K[^"]+' Cargo.toml)"

# What someone who isn't signed in sees: 404 for a private repo, or the login page when the whole server needs signing in.
is_hidden() {
    case "$(curl -s -o /dev/null -m 10 -w '%{http_code} %{redirect_url}' "http://$LAN_HOST/$1")" in
        404\ *|303\ */user/login*) return 0 ;;
        *) return 1 ;;
    esac
}

# ---- git.lan ----
git remote get-url lan >/dev/null 2>&1 || git remote add lan "git@$LAN_HOST:$LAN_CODE.git"
git push lan main "$TAG"
is_hidden "$LAN_CODE" || echo "Note: $LAN_CODE on $LAN_HOST can be seen without signing in. Make it private in Gitea if you want." >&2

# The key goes up in two steps: a commit without it first, to see that the new repo is private.
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
git clone -q "git@$LAN_HOST:$LAN_SIGNING.git" "$work" 2>/dev/null || git -C "$work" init -q -b main
git -C "$work" config user.name "$(git config user.name)"
git -C "$work" config user.email "$(git config user.email)"
git -C "$work" remote get-url origin >/dev/null 2>&1 || git -C "$work" remote add origin "git@$LAN_HOST:$LAN_SIGNING.git"
cat > "$work/README.md" <<'EOF'
# pillion-signing

The release keystore of Pillion: `pillion.jks` (alias `pillion`) and `keystore.properties` with its password.
Installed phones only accept updates signed with this key, so never lose it and never regenerate it.

This repository is private on purpose. Never mirror it anywhere public.

The CI of `Quezka/pillion` has the same key as secrets (`PILLION_KEYSTORE_B64` = base64 of `pillion.jks`,
`PILLION_KEYSTORE_PASSWORD`). To build locally, copy both files into Pillion's `android/` folder.
EOF
git -C "$work" add README.md
git -C "$work" diff --cached --quiet || git -C "$work" commit -q -m "Say what this repository is"
git -C "$work" push -q -u origin main
if ! is_hidden "$LAN_SIGNING"; then
    echo "STOPPED: $LAN_SIGNING on $LAN_HOST is visible without signing in, so the key was not uploaded." >&2
    echo "Make that repo private in Gitea, then run this script again." >&2
    exit 1
fi
cp android/pillion.jks android/keystore.properties "$work/"
git -C "$work" add pillion.jks keystore.properties
git -C "$work" diff --cached --quiet || git -C "$work" commit -q -m "Add the release key"
git -C "$work" push -q origin main
echo "Backed up the code and the signing key to $LAN_HOST."

# ---- GitHub ----
if ! command -v gh >/dev/null || ! gh auth status >/dev/null 2>&1; then
    echo >&2
    echo "GitHub isn't set up on this computer yet. Run these two, then this script again:" >&2
    echo "  sudo apt install gh" >&2
    echo "  gh auth login" >&2
    exit 1
fi
if ! gh repo view "$GITHUB_REPO" >/dev/null 2>&1; then
    gh repo create "$GITHUB_REPO" --private \
        --description "A riding companion for an electric moped: battery, range, spoken directions and a voice mode for riding with an earpiece"
fi
git remote get-url origin >/dev/null 2>&1 || git remote add origin "git@github.com:$GITHUB_REPO.git"

# The secrets go in before the tag, because the tag starts the build that needs them.
base64 -w0 android/pillion.jks | gh secret set PILLION_KEYSTORE_B64 -R "$GITHUB_REPO"
grep -oP '^storePassword=\K.*' android/keystore.properties | tr -d '\n' | gh secret set PILLION_KEYSTORE_PASSWORD -R "$GITHUB_REPO"

git push -u origin main
git push origin "$TAG"

echo
echo "Building. When it finishes, open this on your phone, signed in to GitHub, and download the APK:"
echo "https://github.com/$GITHUB_REPO/releases/latest"
