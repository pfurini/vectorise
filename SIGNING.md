# Signing and notarizing the macOS binaries

An operator runbook. Nothing here happens in CI today, and no credentials are
committed anywhere in this repository.

## What the problem is

A binary downloaded through a browser gets the `com.apple.quarantine` extended
attribute. On first run, Gatekeeper checks it. An unsigned, un-notarized binary
is refused with "cannot be opened because the developer cannot be verified",
and the user has to go into System Settings to allow it.

This affects only downloads that carry the quarantine attribute. `brew install`
and `curl | sh` do not set it, so the Homebrew formula and the shell installer
are unaffected. The people who hit it are those who click a link on the
releases page.

## The cheap fix, for a user

```sh
xattr -d com.apple.quarantine /path/to/vectorise
```

This is in the README, because it is what most people will actually do.

## The real fix, for a maintainer

Signing and notarizing needs an Apple Developer Program membership
(99 USD a year). Without one, none of the following is possible.

### 1. Get a Developer ID Application certificate

In Xcode, **Settings → Accounts → Manage Certificates → + → Developer ID
Application**, or from the Apple Developer portal. Confirm it is installed:

```sh
security find-identity -v -p codesigning
```

The line you want looks like
`Developer ID Application: Your Name (TEAMID1234)`.

### 2. Sign

Sign each architecture's binary, and the universal one separately: `lipo`
strips signatures, so the fat binary has to be signed after it is assembled,
not before.

```sh
codesign \
  --sign "Developer ID Application: Your Name (TEAMID1234)" \
  --options runtime \
  --timestamp \
  --force \
  vectorise

codesign --verify --strict --verbose=2 vectorise
```

`--options runtime` enables the hardened runtime, which notarization requires.
`--timestamp` contacts Apple's timestamp server, so the signature stays valid
after the certificate expires.

### 3. Notarize

Notarization takes an archive, not a bare binary.

```sh
ditto -c -k --keepParent vectorise vectorise.zip

xcrun notarytool submit vectorise.zip \
  --apple-id "you@example.com" \
  --team-id TEAMID1234 \
  --password "$APP_SPECIFIC_PASSWORD" \
  --wait
```

`APP_SPECIFIC_PASSWORD` is an app-specific password from
<https://appleid.apple.com>, not your Apple ID password. Store the credentials
in the keychain once and refer to them by profile name instead:

```sh
xcrun notarytool store-credentials vectorise-notary \
  --apple-id "you@example.com" --team-id TEAMID1234
xcrun notarytool submit vectorise.zip --keychain-profile vectorise-notary --wait
```

If it is rejected:

```sh
xcrun notarytool log <submission-id> --keychain-profile vectorise-notary
```

### 4. Staple

Stapling attaches the notarization ticket to the artifact, so Gatekeeper can
verify it without a network round trip.

A bare Mach-O binary **cannot** be stapled; only bundles, disk images, and
installer packages can. For a plain CLI binary the ticket is fetched online on
first run, which works as long as the machine has a network. If offline
verification matters, ship a `.pkg`:

```sh
pkgbuild --identifier com.example.vectorise --version 0.1.0 \
  --install-location /usr/local/bin --root ./staging vectorise.pkg
productsign --sign "Developer ID Installer: Your Name (TEAMID1234)" \
  vectorise.pkg vectorise-signed.pkg
xcrun notarytool submit vectorise-signed.pkg --keychain-profile vectorise-notary --wait
xcrun stapler staple vectorise-signed.pkg
```

### 5. Verify it worked

On a machine that has never seen the binary:

```sh
spctl --assess --type execute --verbose vectorise
# accepted
# source=Notarized Developer ID
```

## Doing it in CI

Possible, and deliberately not done here. It needs four repository secrets: the
certificate as a base64 `.p12`, its password, the Apple ID, and the
app-specific password. That means a private signing key in a GitHub secret, and
anyone who can run a workflow can sign anything as you.

If it is set up later, the rules are:

- Import the certificate into a **temporary** keychain the job creates and
  deletes, never the default one.
- Never `echo` a secret, including into a debug step.
- Gate the signing job on the tag, so a pull request can never reach it.
- Use `notarytool`, not the retired `altool`.

`cargo-dist` has first-class support for this: set `macos-sign = true` in
`dist-workspace.toml` and provide the secrets it documents. Turning that on is
a decision for whoever owns the Apple Developer account, not for this file.
