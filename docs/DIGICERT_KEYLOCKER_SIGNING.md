# StackDeck protected DigiCert KeyLocker cloud signing

Status: integration code staged. Publisher enrollment and credentials are NOT supplied; public release is NOT authorized.

The implementation uses DigiCert Software Trust Manager/KeyLocker with Windows CNG KSP. The actual code-signing private key stays inside DigiCert's remotely managed protected service. Windows and GitHub CI use a separate, scoped client-authentication certificate, not an exportable signing-key PFX.

## Publisher enrollment requirements

1. Apply for a publicly trusted code-signing certificate in the exact legal publisher name; complete DigiCert identity verification and confirm eligibility for that country.
2. Enroll the public-trust certificate against a remote HSM-protected DigiCert KeyLocker keypair with access granted only to approved signing operators.
3. Obtain a scoped DigiCert ONE client API token and client-authentication P12. Neither represents the code-signing private key.
4. Create a GitHub protected environment named windows-release. Require verified human release approval.
5. Configure settings, run a signed test challenge and independently validate the signature/timestamp on Windows. Record a qualified SHA and evidence digest only after *all* runtime and installer gates pass.

## Protected GitHub environment settings

Secrets: SM_API_KEY, SM_CLIENT_CERT_FILE_B64, SM_CLIENT_CERT_PASSWORD, WINDOWS_SIGNING_CERT_THUMBPRINT.

Variables: SM_HOST, WINDOWS_SIGNING_KEYPAIR_ALIAS, WINDOWS_QUALIFIED_SOURCE_COMMIT, WINDOWS_QUALIFICATION_EVIDENCE_SHA256, WINDOWS_RUNTIME_IMAGE_URL, WINDOWS_RUNTIME_IMAGE_SHA512, WINDOWS_DEBIAN_SNAPSHOT_TIMESTAMP, WINDOWS_NERDCTL_VERSION, WINDOWS_NERDCTL_SHA256_AMD64.

SM_CLIENT_CERT_FILE_B64 is the base64-encoded client authentication P12, NEVER a public-trust signing private-key export.

## Signing workflow gates

- The release workflow refuses to proceed when any KeyLocker credential is missing.
- The official DigiCert GitHub Action installs the Software Trust Manager tooling.
- The prep script registers DigiCert KSP and synchronizes only the named signing alias.
- It requires a matching trusted public CA chain, valid certificate, Code Signing EKU, and a DigiCert KSP-backed private-key handle.
- Before packaging, the release script signs executable sidecars and desktop binary. After packaging, it signs MSI and NSIS. Hashes, signatures and timestamps are independently verified.
- The exact Git commit and independent acceptance-evidence digest are required for any production tag.
- The temporary client-authentication certificate file is removed even on workflow failure.

## Safety constraints

Never import a signing private key into GitHub Actions as a PFX. Never store vendor API keys, client P12, identity documents or passwords in source. A current/self-signed PyStack development certificate is disallowed.

A certificate's Windows store visibility alone does not prove real HSM custody. Obtain DigiCert-issued evidence of remote key storage and verify the timestamped Authenticode signing challenge against an independently trusted Windows installation.

Microsoft Azure Artifact Signing Public Trust is subject to country eligibility. Its Private Trust option is not a substitute for a publicly trusted Authenticode publisher identity.

Release state remains NO-GO until the publisher enrolls, the corrected Debian guest boots with SSH/OCI/Compose, Windows install-upgrade tests pass and all acceptance evidence is approved.
