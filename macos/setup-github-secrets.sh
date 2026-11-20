#!/bin/sh
# Gives GitHub what it needs to sign and notarise the macOS release with your Apple Developer
# membership, as encrypted repository secrets. Run it once on your Mac, in this folder, after the
# repository is on GitHub:
#
#   sh macos/setup-github-secrets.sh            # the repository of this folder
#   sh macos/setup-github-secrets.sh owner/repo # another one
#   sh macos/setup-github-secrets.sh --dry-run  # check everything, upload nothing
#
# Needs: your "Developer ID Application" certificate in the login keychain (Xcode > Settings >
# Accounts > Manage Certificates creates it), the GitHub CLI logged in (`gh auth login`), and an
# app-specific password for your Apple Account (account.apple.com > Sign-In and Security >
# App-Specific Passwords). macOS asks once to let `security` export the certificate's key: choose
# "Allow". Only the Developer ID certificate is uploaded, nothing else from your keychain, and
# nothing secret is printed or left on disk.
set -eu

DRY=0
REPO=""
for a in "$@"; do
    case "$a" in
        --dry-run) DRY=1 ;;
        *) REPO="$a" ;;
    esac
done
OPENSSL=/usr/bin/openssl   # LibreSSL: its .p12 files import everywhere `security` runs

if [ "$DRY" = 0 ]; then
    command -v gh >/dev/null 2>&1 || { echo "Install the GitHub CLI first (https://cli.github.com, or: brew install gh), then: gh auth login" >&2; exit 1; }
    [ -n "$REPO" ] || REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner)" || { echo "Run this inside the repository folder, or name the repository: owner/repo" >&2; exit 1; }
fi

# the certificate
LINE="${ARBOR_TEST_IDENTITY:-$(security find-identity -v -p codesigning | grep '"Developer ID Application' | head -1 || true)}"
if [ -z "$LINE" ]; then
    echo "No 'Developer ID Application' certificate in your keychain. Create one in Xcode > Settings > Accounts > Manage Certificates (+ > Developer ID Application) and run this again." >&2
    exit 1
fi
HASH="$(echo "$LINE" | awk '{print $2}')"
NAME="$(echo "$LINE" | sed -E 's/^[^"]*"(.*)"[^"]*$/\1/')"
TEAM="$(echo "$NAME" | sed -nE 's/.*\(([A-Z0-9]{10})\)$/\1/p')"
echo "Certificate: $NAME"
[ -n "$REPO" ] && echo "Repository:  $REPO"

WORK="$(mktemp -d)"
chmod 700 "$WORK"
trap 'rm -rf "$WORK"' EXIT INT TERM
P12_PASS="$($OPENSSL rand -base64 24)"
export P12_PASS

# Export the keychain's identities, then keep only the Developer ID one.
if [ -n "${ARBOR_TEST_P12:-}" ]; then
    cp "$ARBOR_TEST_P12" "$WORK/all.p12"   # self-test with a made-up keychain export
    P12_IN_PASS="${ARBOR_TEST_P12_PASS:-}"
else
    security export -k login.keychain-db -t identities -f pkcs12 -P "$P12_PASS" -o "$WORK/all.p12" >/dev/null
    P12_IN_PASS="$P12_PASS"
fi
export P12_IN_PASS
$OPENSSL pkcs12 -in "$WORK/all.p12" -passin env:P12_IN_PASS -nokeys -clcerts -out "$WORK/certs.pem" 2>/dev/null
$OPENSSL pkcs12 -in "$WORK/all.p12" -passin env:P12_IN_PASS -nocerts -nodes -out "$WORK/keys.pem" 2>/dev/null
awk -v d="$WORK" '/-----BEGIN CERTIFICATE-----/{n++; f=d "/cert" n ".pem"} f{print > f} /-----END CERTIFICATE-----/{f=""}' "$WORK/certs.pem"
awk -v d="$WORK" '/-----BEGIN (RSA |EC |ENCRYPTED )?PRIVATE KEY-----/{n++; f=d "/key" n ".pem"} f{print > f} /-----END (RSA |EC |ENCRYPTED )?PRIVATE KEY-----/{f=""}' "$WORK/keys.pem"
CERT=""
for c in "$WORK"/cert*.pem; do
    [ -f "$c" ] || continue
    fp="$($OPENSSL x509 -in "$c" -noout -fingerprint -sha1 | sed -e 's/.*=//' -e 's/://g')"
    if [ "$fp" = "$HASH" ]; then CERT="$c"; fi
done
[ -n "$CERT" ] || { echo "The Developer ID certificate was not in the export (was access allowed?)." >&2; exit 1; }
PUB="$($OPENSSL x509 -in "$CERT" -noout -pubkey)"
KEY=""
for k in "$WORK"/key*.pem; do
    [ -f "$k" ] || continue
    if [ "$($OPENSSL pkey -in "$k" -pubout 2>/dev/null)" = "$PUB" ]; then KEY="$k"; fi
done
[ -n "$KEY" ] || { echo "The Developer ID certificate's private key was not in the export (was access allowed?)." >&2; exit 1; }
# Apple's Developer ID intermediate certificates go along, so the build machine trusts the chain
security find-certificate -a -c "Developer ID Certification Authority" -p \
    /Library/Keychains/System.keychain /System/Library/Keychains/SystemRootCertificates.keychain \
    > "$WORK/chain.pem" 2>/dev/null || true
if [ -s "$WORK/chain.pem" ]; then
    $OPENSSL pkcs12 -export -in "$CERT" -inkey "$KEY" -certfile "$WORK/chain.pem" -name "$NAME" -out "$WORK/developer-id.p12" -passout env:P12_PASS
else
    $OPENSSL pkcs12 -export -in "$CERT" -inkey "$KEY" -name "$NAME" -out "$WORK/developer-id.p12" -passout env:P12_PASS
fi
$OPENSSL pkcs12 -in "$WORK/developer-id.p12" -passin env:P12_PASS -noout 2>/dev/null || { echo "Could not repack the certificate." >&2; exit 1; }
echo "Certificate and key packed (only the Developer ID one)."

if [ "$DRY" = 1 ]; then
    echo "Dry run: would set MACOS_CERTIFICATE, MACOS_CERTIFICATE_PASSWORD, APPLE_TEAM_ID${TEAM:+ ($TEAM)}, APPLE_ID, APPLE_APP_PASSWORD."
    exit 0
fi

# Apple Account for notarisation
printf "Apple Account email of the developer membership: "
read -r APPLE_ID
printf "App-specific password (typing is hidden): "
stty -echo
read -r APPLE_APP_PASSWORD
stty echo
echo
if [ -z "$TEAM" ]; then
    printf "Team ID (10 characters, developer.apple.com > Account > Membership details): "
    read -r TEAM
fi
echo "Checking the Apple Account with Apple's notary service..."
if ! xcrun notarytool history --apple-id "$APPLE_ID" --password "$APPLE_APP_PASSWORD" --team-id "$TEAM" >/dev/null 2>&1; then
    echo "Apple did not accept these details (email, app-specific password or team id). Nothing was uploaded." >&2
    exit 1
fi

base64 -i "$WORK/developer-id.p12" | gh secret set MACOS_CERTIFICATE --repo "$REPO"
printf %s "$P12_PASS" | gh secret set MACOS_CERTIFICATE_PASSWORD --repo "$REPO"
printf %s "$APPLE_ID" | gh secret set APPLE_ID --repo "$REPO"
printf %s "$APPLE_APP_PASSWORD" | gh secret set APPLE_APP_PASSWORD --repo "$REPO"
printf %s "$TEAM" | gh secret set APPLE_TEAM_ID --repo "$REPO"
echo "Done. The next release (a v... tag) is signed and notarised by Apple."
