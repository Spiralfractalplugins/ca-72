# A trusted macOS release

The public macOS installer must contain Developer ID Application-signed VST3, CLAP and
Audio Unit bundles (the Audio Unit carries the same signed CLAP inside it), be signed with
Developer ID Installer, and be accepted by Apple's notary service. The notarization ticket is stapled to the installer so it can be verified
without first contacting Apple.

The release identities are:

- `Developer ID Application: Idle Foundry Ltd. (3JA8JUZ36W)`
- `Developer ID Installer: Idle Foundry Ltd. (3JA8JUZ36W)`

Their private keys are kept in the releasing Mac's login Keychain, with encrypted backups
outside the repository. Never commit private keys, passwords, or signing archives.

## One-time notarization credentials

In Apple Account, create an app-specific password named CA-72 Notarization, then enter it
at the secure prompt from:

```sh
xcrun notarytool store-credentials CA72_NOTARY \
  --apple-id YOUR_APPLE_ACCOUNT_EMAIL --team-id 3JA8JUZ36W
```

The password is saved in Keychain. Do not pass it on the command line or put it in a
shell configuration file.

## Build and verify

From a clean checkout of the version to release (once:
`rustup target add x86_64-apple-darwin aarch64-apple-darwin` and `brew install cmake`):

```sh
export CA72_PUBLIC_RELEASE=1
cargo run -p ca72-panel --example check_release_key
cargo xtask bundle-universal ca72-plugin --profile bundle
scripts/auv2.sh
scripts/standalone.sh
export CA72_MACOS_SIGN_APP='Developer ID Application: Idle Foundry Ltd. (3JA8JUZ36W)'
export CA72_MACOS_SIGN_INSTALLER='Developer ID Installer: Idle Foundry Ltd. (3JA8JUZ36W)'
export CA72_NOTARY_PROFILE=CA72_NOTARY
export CA72_REQUIRE_NOTARIZATION=1
scripts/package.sh
```

Before the 0.3.0 release candidate, fill the first-party skin public key as described in
[Signing first-party packs](skin-packs.md#signing-first-party-packs). With
`CA72_PUBLIC_RELEASE=1` an empty slot blocks the build; the diagnostic above reports it first.

The four exports may be kept in a local file outside the repository and loaded with
`source`; it holds identity names and the Keychain profile's name, never the notarization
password.

With `CA72_REQUIRE_NOTARIZATION=1`, missing signing credentials stop packaging. The
script checks that Apple's result is Accepted, staples and validates the ticket, verifies
the installer certificate chain, and requires Gatekeeper to accept the final installer.
Only after those checks pass does the script replace the distributable in
`target/packages/`. Keep `target/packaging/notarization.json` with the release evidence.

Distribute the verified `.pkg` from `target/packages/`. The raw bundles in
`target/bundled/` are development artifacts; the signed payload is inside the installer.
The installer includes the universal standalone app in Applications. Its hardened
signature carries the audio-input entitlement, and its Info.plist explains optional
microphone access; output-only playback does not request microphone permission.
The existing hosted CI artifacts are unsigned until signing secrets are separately
configured there. Public macOS releases must use the verified installer produced above.
Normal Installer confirmation and administrator-password prompts still apply.
