#Requires -Version 5.1
<#
.SYNOPSIS
  Fail-closed DigiCert KeyLocker KSP initialization for protected Windows releases.
.DESCRIPTION
  Uses a CA-verified signing certificate synchronized into CurrentUser\My
  while its SIGNING private key stays in DigiCert's remote KeyLocker HSM.
  SM_CLIENT_CERT_FILE is only the KeyLocker CLIENT AUTHENTICATION credential.
  Never substitutes it for the public Authenticode signing certificate.
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory=$true)][ValidatePattern('^[a-fA-F0-9]{40}$')]
  [string]$CertificateThumbprint,
  [Parameter(Mandatory=$true)][ValidateNotNullOrEmpty()]
  [string]$KeypairAlias
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
foreach($var in @('SM_HOST','SM_API_KEY','SM_CLIENT_CERT_FILE','SM_CLIENT_CERT_PASSWORD')) {
  if([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($var,'Process'))) {
    throw "KeyLocker authentication requirement $var is missing."
  }
}
if(-not (Test-Path -LiteralPath $env:SM_CLIENT_CERT_FILE -PathType Leaf)) {
  throw 'KeyLocker client authentication certificate file is unavailable.'
}
$u=[uri]$env:SM_HOST
if($u.Scheme -ne 'https'){throw 'SM_HOST must be a validated HTTPS DigiCert endpoint.'}
$smctl=Get-Command smctl.exe -ErrorAction SilentlyContinue | Select-Object -First 1
if(-not $smctl){$smctl=Get-Command smctl -ErrorAction SilentlyContinue |Select-Object -First 1}
if(-not $smctl){throw 'DigiCert SMCTL client tools were not installed by the approved setup action.'}
$target=$CertificateThumbprint.ToUpperInvariant()
# Registration of the provider requires an elevated runner; it never generates
# or imports a signing private key on this host.
& $smctl.Source windows ksp register
if($LASTEXITCODE -ne 0){throw 'KeyLocker KSP registration failed.'}
& $smctl.Source windows certsync --keypair-alias $KeypairAlias
if($LASTEXITCODE -ne 0){throw 'KeyLocker signing certificate synchronization failed.'}
$certs=@(Get-ChildItem Cert:\CurrentUser\My -ErrorAction Stop |
  Where-Object { $_.Thumbprint -ieq $target })
if($certs.Count -ne 1){throw 'Expected public-trust signing certificate was not synchronized.'}
$cert=$certs[0]
if(-not $cert.HasPrivateKey){throw 'KSP-backed private-key handle is unavailable.'}
if($cert.Subject -eq $cert.Issuer){throw 'Self-signed code signing certificate is prohibited.'}
if($cert.NotBefore -gt (Get-Date) -or $cert.NotAfter -le (Get-Date)){throw 'Signing certificate is not currently valid.'}
if(@($cert.EnhancedKeyUsageList | ForEach-Object {[string]$_.ObjectId}) -notcontains '1.3.6.1.5.5.7.3.3'){
  throw 'Certificate does not have Code Signing EKU.'
}
$chain=[Security.Cryptography.X509Certificates.X509Chain]::new()
$chain.ChainPolicy.RevocationMode=[Security.Cryptography.X509Certificates.X509RevocationMode]::Online
$chain.ChainPolicy.RevocationFlag=[Security.Cryptography.X509Certificates.X509RevocationFlag]::ExcludeRoot
$chain.ChainPolicy.VerificationFlags=[Security.Cryptography.X509Certificates.X509VerificationFlags]::NoFlag
if(-not $chain.Build($cert)){throw 'CA trust or online revocation chain validation failed.'}
$leaf=(@($chain.ChainElements)|Select-Object -Last 1).Certificate
if($leaf.Subject -ne $leaf.Issuer){throw 'Expected a trusted CA root in the verified chain.'}
if(-not (@($chain.ChainElements | Where-Object {
  $_.Certificate.Subject -match 'DigiCert' -or $_.Certificate.Issuer -match 'DigiCert'
}).Count)) {throw 'DigiCert issuance was not found in the validated trust chain.'}
# Fail closed if the signing private-key handle is not DigiCert KSP.
# certutil outputs the provider for the selected synchronized cert.
$details=(& certutil.exe -user -store My $target 2>&1 | Out-String)
if($LASTEXITCODE -ne 0 -or $details -notmatch 'DigiCert[^\r\n]*KSP'){
  throw 'Certificate does not expose the expected DigiCert KeyLocker KSP provider.'
}
Write-Output ('KEYLOCKER_PUBLIC_CERT_VALIDATED='+$target)
Write-Output 'CLOUD_KEY_REMOTES_ONLY=True'
if($env:GITHUB_OUTPUT){
  ('thumbprint='+$target) | Out-File -FilePath $env:GITHUB_OUTPUT -Encoding utf8 -Append
}
