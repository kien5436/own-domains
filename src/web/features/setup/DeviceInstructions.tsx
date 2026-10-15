export function DeviceInstructions({ lanAddress }: { lanAddress: string }) {
  return <section aria-labelledby="device-heading" className="rounded-2xl border border-slate-200 bg-white p-5 shadow-sm">
    <h2 className="text-lg font-semibold" id="device-heading">Phone setup</h2>
    <ol className="mt-3 list-decimal space-y-2 pl-5 text-sm text-slate-600">
      <li>Reserve the Mac address <strong className="text-slate-900">{lanAddress || 'Not configured'}</strong> in your router.</li>
      <li>Set that address as the phone Wi-Fi network's only DNS server.</li>
      <li>Install the public local CA certificate and enable full certificate trust for HTTPS.</li>
      <li>Restore automatic DNS whenever this Mac is unavailable.</li>
    </ol>
  </section>;
}
