import { expect, it, vi } from 'vitest';
import { createNativeClient, type NativeTransport } from '../../../src/web/lib/native-client';
import type { EventEnvelope } from '../../../src/web/lib/native-protocol';

it('buffers events delivered before the subscription ID and removes listeners on cleanup', async () => {
  const handlers = new Map<string, (event: { payload: unknown }) => void>();
  const removed: string[] = [];
  const event: EventEnvelope = { protocolVersion: 1, sequence: 4, event: { type: 'settingsChanged', payload: { revision: 4 } } };
  const transport: NativeTransport = {
    listen(name, handler) {
      handlers.set(name, handler as (event: { payload: unknown }) => void);

      return Promise.resolve(() => { removed.push(name) });
    },
    invoke<T>(name: string, args?: Record<string, unknown>) {
      if ('daemon_subscribe' === name) {
        expect(args).toEqual({ afterSequence: 3 });
        expect([...handlers.keys()]).toEqual(['daemon-event', 'daemon-disconnected']);
        handlers.get('daemon-event')!({ payload: { subscriptionId: 7, envelope: event } });

        return Promise.resolve(7 as T);
      }

      expect(name).toBe('daemon_unsubscribe'); expect(args).toEqual({ subscriptionId: 7 });

      return Promise.resolve(undefined as T);
    },
  };
  const received = vi.fn(); const disconnected = vi.fn();
  const stop = await createNativeClient(transport).subscribe(3, received, disconnected);

  expect(received).toHaveBeenCalledWith(event);
  handlers.get('daemon-event')!({ payload: { subscriptionId: 6, envelope: event } });
  expect(received).toHaveBeenCalledTimes(1);
  await stop(); expect(removed).toEqual(['daemon-event', 'daemon-disconnected']);
  handlers.get('daemon-event')!({ payload: { subscriptionId: 7, envelope: event } });
  expect(received).toHaveBeenCalledTimes(1);
});
it('cleans up installed listeners when starting a subscription fails', async () => {
  const cleanup = vi.fn();
  const transport: NativeTransport = { listen: () => Promise.resolve(cleanup), invoke: () => Promise.reject(new Error('unavailable')) };

  await expect(createNativeClient(transport).subscribe(null, vi.fn(), vi.fn())).rejects.toThrow('unavailable');
  expect(cleanup).toHaveBeenCalledTimes(2);
});
