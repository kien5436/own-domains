// @vitest-environment jsdom
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { NativeApp } from '../../src/web/app/NativeApp';
import { RuntimeStore } from '../../src/web/features/runtime/runtime-store';
import type { NativeClient } from '../../src/web/lib/native-client';
import type { EventEnvelope, StateSnapshot } from '../../src/web/lib/native-protocol';

it('manages domains and network through IPC without HTTP and shows stopped truthfully', async () => {
  const fetch = vi.spyOn(globalThis, 'fetch').mockRejectedValue(new Error('Native management must not use HTTP'));
  let state: StateSnapshot = { settings: { schemaVersion: 1, revision: 0, lanAddress: '', upstreamDns: ['1.1.1.1'], mappings: [] }, runtimeStatus: 'stopped', desiredRevision: 0, appliedRevision: 0, diagnostics: [], latestEventSequence: 0 };
  let emit!: (event: EventEnvelope) => void;
  const client: NativeClient = {
    subscribe: async (_after, onEvent) => {
      emit = onEvent;

      return async () => {};
    },
    request: vi.fn<NativeClient['request']>(async (command) => {
      if (command.type === 'snapshot') return { type: 'snapshot', payload: structuredClone(state) };
      if (command.type === 'create') state.settings.mappings.push({ ...command.payload.input, id: 'one' });
      if (command.type === 'update') Object.assign(state.settings.mappings[0], command.payload.patch);
      if (command.type === 'remove') state.settings.mappings = [];

      if (command.type === 'updateNetwork') { state.settings.lanAddress = command.payload.lan_address; state.settings.upstreamDns = command.payload.upstream_dns }

      state.settings.revision++; state.desiredRevision++; state.latestEventSequence++;
      const apply = command.type === 'updateNetwork' ? { desiredRevision: state.desiredRevision, appliedRevision: state.appliedRevision, status: 'error' as const, message: 'reload failed' } : null;

      if (apply) state.runtimeStatus = 'degraded';

      return { type: 'mutation', payload: { settings: structuredClone(state.settings), apply } };
    }),
  };
  const user = userEvent.setup(); const store = new RuntimeStore(client);
  const view = render(<NativeApp store={store} onStart={async () => { throw new Error('Runtime unavailable') }} onQuit={async () => {}} />);

  await screen.findByText('No domains registered yet.');
  expect(screen.getByText('Runtime: stopped')).toBeInTheDocument();
  expect(screen.queryByText(/Configuration applied/)).not.toBeInTheDocument();
  await user.type(screen.getByLabelText('Domain'), 'shop'); await user.click(screen.getByRole('button', { name: 'Register domain' }));
  await screen.findByText('shop.own');
  await user.click(screen.getByRole('button', { name: 'Disable' }));
  await screen.findByRole('button', { name: 'Enable' });
  await user.click(screen.getByRole('button', { name: 'Turn TLS on' }));
  await screen.findByText('127.0.0.1:3000 · HTTPS');
  await user.click(screen.getByRole('button', { name: 'Edit shop.own' }));
  await user.clear(screen.getByLabelText('Upstream port for shop.own'));
  await user.type(screen.getByLabelText('Upstream port for shop.own'), '4000');
  await user.click(screen.getByRole('button', { name: 'Save shop.own' }));
  await screen.findByText('127.0.0.1:4000 · HTTPS');
  const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);

  await user.click(screen.getByRole('button', { name: 'Delete' }));
  await screen.findByText('No domains registered yet.');
  confirm.mockRestore();
  await user.type(screen.getByLabelText('Mac LAN IPv4'), '192.168.1.20'); await user.click(screen.getByRole('button', { name: 'Save network settings' }));
  await waitFor(() => expect(state.settings.lanAddress).toBe('192.168.1.20'));
  await screen.findByText('Runtime: degraded');
  expect(screen.getByText(/pending: desired 6, applied 0/)).toBeInTheDocument();
  expect(screen.getByRole('alert')).toHaveTextContent('reload failed');
  state = { ...state, settings: { ...state.settings, revision: 9, mappings: [{ id: 'cli', hostname: 'cli.own', upstreamPort: 8080, enabled: true, tls: false }] }, desiredRevision: 9, latestEventSequence: 9 };
  emit({ protocolVersion: 2, sequence: 9, event: { type: 'settingsChanged', payload: { revision: 9 } } });
  await screen.findByText('cli.own'); expect(screen.getByText('Revision 9')).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: 'Start routing' }));
  await screen.findByText('Runtime unavailable');
  expect(fetch).not.toHaveBeenCalled(); view.unmount(); fetch.mockRestore();
});
