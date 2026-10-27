# Signing the Windows build

Smart App Control (on by default in new Windows 11 installs) blocks unsigned programs, so the Windows zip only
runs for most people once its `.exe` files are signed. The release workflow signs them with
[Azure Artifact Signing](https://learn.microsoft.com/azure/artifact-signing/) when it's set up, and leaves them
unsigned otherwise.

Cost: the Basic plan is $9.99 a month. Individual developers must be in the US or Canada; your identity is
checked before the certificate is issued (allow a few business days).

## One-time setup (Azure portal)

1. **Subscription and resource provider.** In an Azure subscription, register the `Microsoft.CodeSigning`
   resource provider (Subscriptions → your subscription → Resource providers).
2. **Artifact Signing account.** Create an Artifact Signing account (Basic). Note its **region endpoint**, e.g.
   `https://eus.codesigning.azure.net/` for East US, and its **name**.
3. **Identity validation.** In the account, start an *Individual* identity validation (your user needs the
   account's *Identity Verifier* role first). Wait for it to complete.
4. **Certificate profile.** Create a *Public Trust* certificate profile from the completed validation. Note its
   **name**.
5. **App registration for GitHub.** In Microsoft Entra ID → App registrations, register an app (e.g.
   `cranked-audio-release`). Under *Certificates & secrets → Federated credentials*, add a GitHub Actions
   credential: organization `Zach9113`, repository `cranked-audio`, entity type *Environment*, environment
   `release` (the Windows job runs in that environment; federated credentials don't take tag wildcards). No client
   secret is needed.
6. **Permission.** On the Artifact Signing account (or the certificate profile), give that app the
   **Artifact Signing Certificate Profile Signer** role.

## GitHub (repository settings)

Environment: Settings → Environments → *New environment* `release` (optionally limit it to tags `v*` under
*Deployment branches and tags*).

Secrets (Settings → Secrets and variables → Actions → *Secrets*):

| Name | Value |
|---|---|
| `AZURE_CLIENT_ID` | the app registration's Application (client) ID |
| `AZURE_TENANT_ID` | the Directory (tenant) ID |
| `AZURE_SUBSCRIPTION_ID` | the subscription ID |

Variables (same page → *Variables*):

| Name | Value |
|---|---|
| `ARTIFACT_SIGNING_ENDPOINT` | the account's region endpoint, e.g. `https://eus.codesigning.azure.net/` |
| `ARTIFACT_SIGNING_ACCOUNT` | the Artifact Signing account name |
| `ARTIFACT_SIGNING_PROFILE` | the certificate profile name |

Setting `ARTIFACT_SIGNING_ACCOUNT` turns signing on. The next tagged release (or a manual run of the Release
workflow) signs `cranked-audiod.exe` and `cranked-audio.exe` before they're zipped.

## Checking a signed build

```powershell
Get-AuthenticodeSignature "$env:LOCALAPPDATA\Programs\Cranked Audio\cranked-audiod.exe" | Format-List Status, SignerCertificate
```

`Status` should be `Valid`. A new publisher may still meet SmartScreen warnings until it builds reputation;
signing every release with the same identity builds it.
