export function DeviceInstructions({ lanAddress }: { lanAddress: string }) {
  return <div className="grid gap-6">
    <section aria-labelledby="phone-heading" className="rounded-2xl border border-slate-200 bg-white p-5 shadow-sm dark:border-slate-700 dark:bg-slate-900">
      <h2 className="text-lg font-semibold" id="phone-heading">Phone setup</h2>
      <ol className="mt-3 list-decimal space-y-2 pl-5 text-sm text-slate-600 dark:text-slate-300">
        <li>Reserve the Mac address <strong className="text-slate-900 dark:text-white">{lanAddress || 'Not configured'}</strong> in your router.</li>
        <li>Set that address as the phone Wi-Fi network's only DNS server.</li>
        <li>Install the public local CA certificate and enable full certificate trust for HTTPS.</li>
        <li>Restore automatic DNS whenever this Mac is unavailable.</li>
      </ol>
    </section>
    <section aria-labelledby="certificate-heading" className="rounded-2xl border border-slate-200 bg-white p-5 shadow-sm dark:border-slate-700 dark:bg-slate-900">
      <h2 className="text-lg font-semibold" id="certificate-heading">Certificate trust</h2>
      <ol className="mt-3 list-decimal space-y-2 pl-5 text-sm text-slate-600 dark:text-slate-300">
        <li>Open <strong className="text-slate-900 dark:text-white">http://setup.own</strong> on the phone and download the CA certificate.</li>
        <li>On iOS, install the profile, then enable full trust under Settings → General → About → Certificate Trust Settings.</li>
        <li>On Android, install the CA under Settings → Security → Encryption &amp; credentials, then confirm HTTPS loads without warnings.</li>
        <li>Never copy the CA private key off this machine.</li>
      </ol>
    </section>
    <section aria-labelledby="troubleshooting-heading" className="rounded-2xl border border-slate-200 bg-white p-5 shadow-sm dark:border-slate-700 dark:bg-slate-900">
      <h2 className="text-lg font-semibold" id="troubleshooting-heading">Troubleshooting</h2>
      <ul className="mt-3 list-disc space-y-2 pl-5 text-sm text-slate-600 dark:text-slate-300">
        <li>Domain does not resolve: confirm the phone uses this machine as its only DNS server and is not on an isolated guest network or a VPN with private DNS.</li>
        <li>502 response: start the app on the configured local port; some dev servers must allow the custom .own Host header.</li>
        <li>HTTPS warning: the CA certificate is installed but not trusted — complete the trust step above for your platform.</li>
        <li>Works here but not on the phone: check firewall access to TCP 80/443 and UDP/TCP 53, then confirm the LAN address has not changed.</li>
      </ul>
    </section>
  </div>;
}
