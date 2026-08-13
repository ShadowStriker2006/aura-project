# Publishing Aura for Windows

End users should download and run only `Aura_<version>_x64-setup.exe`. They do
not need this source tree, Rust, Cargo, PowerShell, or `BUILD-WINDOWS.bat`.

## Automated release flow

1. Put the project in a GitHub repository.
2. Make the version identical in `src-tauri/Cargo.toml` and
   `src-tauri/tauri.conf.json`.
3. Configure Windows signing as described below.
4. Push a semantic version tag matching the application, such as `v0.14.0`.
5. `.github/workflows/release-windows.yml` validates the version, restores the
   Rust build cache, runs release tests, builds Windows x64, creates the NSIS
   installer in a private draft, verifies its signature, generates
   `SHA256SUMS.txt`, and only then publishes both files on the GitHub Release.

The workflow can also be started manually. Manual runs default to a draft and
may build unsigned test artifacts. A tag-triggered public release refuses to
publish unless signing is fully configured and the final installer signature is
valid.

## Required GitHub signing configuration

Create an Authenticode code-signing certificate through a trusted certificate
provider. Add these protected GitHub repository settings:

- Secret `WINDOWS_CERTIFICATE`: the Base64-encoded PFX bytes.
- Secret `WINDOWS_CERTIFICATE_PASSWORD`: the PFX export password.
- Variable `WINDOWS_TIMESTAMP_URL`: the certificate issuer's absolute HTTPS
  timestamp URL.

The workflow imports the certificate only into the temporary runner account and
removes the certificate and temporary files in an `always()` cleanup step. No
certificate or password belongs in source control.

## Faster repeat builds

- GitHub restores Cargo dependencies and compiled outputs with `rust-cache`.
- The release profile uses thin LTO, eight code-generation units, and incremental
  compilation. This keeps runtime optimization while avoiding the previous
  single-unit full-LTO rebuild bottleneck.
- The bundle configuration builds NSIS only instead of preparing unused bundle
  formats.
- Local publisher builds reuse the same target directory and explicit Windows
  x64 target.
- `BUILD-WINDOWS.bat` fingerprints all executable inputs and tool versions. If
  they match an exact current-version executable and installer, it only stages
  and re-hashes those files. Any source, config, dependency, icon, Rust, or
  Tauri CLI change automatically triggers a real rebuild.

Do not delete `src-tauri/target` between normal repeat builds. Use a clean build
only when diagnosing a compiler or dependency problem.

## Riot production requirement

The current desktop credential flow is safe for personal development because a
user-supplied Riot key stays in Windows Credential Manager and never enters the
webview. It is not a substitute for public production infrastructure.

Before distributing Aura publicly:

1. Register the product with Riot and obtain production approval.
2. Host Match-V5 calls behind an approved server-side API so the production key
   is never distributed in the desktop executable.
3. Add a website on a domain you control with the product description, download,
   Terms of Service, and Privacy Policy.
4. Implement server-side rate-limit handling, monitoring, abuse controls, and
   key rotation.

Never place a Riot production key in `tauri.conf.json`, Rust constants, frontend
JavaScript, GitHub Release assets, or installer command-line arguments.

## Spotify publication requirement

Aura Player uses Spotify's official Web Playback SDK and is limited to Spotify
Premium accounts. Spotify's current platform rules restrict commercial
streaming applications. Review the current Spotify Developer Policy and obtain
written approval where required before publishing a commercial build. Keep the
fresh-session browser fallback available and test protected playback in WebView2
and current Edge or Chrome on clean Windows 10 and Windows 11 systems.

## Release checklist

- Choose and add the intended software license.
- Update the changelog and application version.
- Run formatting, strict linting, tests, and the native smoke test.
- Confirm the installer is Authenticode-signed and timestamped.
- Compare the published SHA-256 checksum with the locally downloaded installer.
- Test install, launch, upgrade, and uninstall on a clean Windows user account.
- Verify the Riot production service and Spotify redirect URI from the public
  build without exposing credentials in logs.
- Verify Aura Player startup, activation, playback, token refresh, and fallback
  behavior with a permitted Premium test account.
- Expand a recent Summoner's Rift report and verify timeline load, playback,
  scrubbing, objective banners, collapse/reopen cleanup, and the estimated-
  control disclaimer with an approved Riot production service.
