import { createHash } from 'node:crypto';

export function buildAppleCaProfile(pemCertificate: string): string {
  const certificate = certificateDer(pemCertificate);
  const certificateId = createHash('sha256').update(certificate).digest('hex');
  const profileIdentifier = `dev.owndomains.ca.${certificateId.slice(0, 16)}`;
  const certificateUuid = stableUuid('certificate', certificate);
  const profileUuid = stableUuid('profile', certificate);

  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>PayloadContent</key>
  <array>
    <dict>
      <key>PayloadCertificateFileName</key>
      <string>own-domains-ca.cer</string>
      <key>PayloadContent</key>
      <data>${certificate.toString('base64')}</data>
      <key>PayloadDescription</key>
      <string>Trusts HTTPS certificates issued for local .own domains.</string>
      <key>PayloadDisplayName</key>
      <string>Own Domains Local CA</string>
      <key>PayloadIdentifier</key>
      <string>${profileIdentifier}.certificate</string>
      <key>PayloadType</key>
      <string>com.apple.security.root</string>
      <key>PayloadUUID</key>
      <string>${certificateUuid}</string>
      <key>PayloadVersion</key>
      <integer>1</integer>
    </dict>
  </array>
  <key>PayloadDescription</key>
  <string>Installs the Own Domains local root certificate.</string>
  <key>PayloadDisplayName</key>
  <string>Own Domains Local CA</string>
  <key>PayloadIdentifier</key>
  <string>${profileIdentifier}</string>
  <key>PayloadOrganization</key>
  <string>Own Domains</string>
  <key>PayloadType</key>
  <string>Configuration</string>
  <key>PayloadUUID</key>
  <string>${profileUuid}</string>
  <key>PayloadVersion</key>
  <integer>1</integer>
</dict>
</plist>
`;
}

function certificateDer(pemCertificate: string): Buffer {
  const match = pemCertificate.trim().match(/^-----BEGIN CERTIFICATE-----\s+([A-Za-z\d+/=\s]+?)\s+-----END CERTIFICATE-----$/);

  if (!match) throw new Error('A PEM certificate is required');
  const encoded = match[1]!.replaceAll(/\s/g, '');
  const certificate = Buffer.from(encoded, 'base64');

  if (!certificate.length || certificate.toString('base64').replaceAll('=', '') !== encoded.replaceAll('=', '')) throw new Error('A valid PEM certificate is required');

  return certificate;
}

function stableUuid(purpose: string, certificate: Buffer): string {
  const hex = createHash('sha256').update(purpose).update(certificate).digest('hex').slice(0, 32).toUpperCase();

  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}
