import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

import type { EventEnvelope, NativeCommand, NativeCommandResult } from './native-protocol';

export type NativeClient = {
  request: (command: NativeCommand) => Promise<NativeCommandResult>;
  subscribe: (afterSequence: number | null, onEvent: (event: EventEnvelope) => void, onDisconnect: (error: unknown) => void) => Promise<() => Promise<void>>;
};

export type NativeTransport = {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  listen: <T>(event: string, handler: (event: { payload: T }) => void) => Promise<() => void>;
};

export function createNativeClient(transport: NativeTransport = { invoke, listen }): NativeClient {
  return {
    request: (command) => transport.invoke<NativeCommandResult>('daemon_request', { command }),
    async subscribe(afterSequence, onEvent, onDisconnect) {
      type Message = { subscriptionId: number; envelope?: EventEnvelope; error?: unknown };
      let id: number | null = null;
      let active = true;
      const queued: Message[] = [];
      const cleanups: (() => void)[] = [];

      const deliver = (message: Message) => {
        if (!active) return;

        if (null === id) {
          queued.push(message);

          return;
        }

        if (message.subscriptionId !== id) return;
        if (message.envelope)
          if (1 !== message.envelope.protocolVersion) onDisconnect(new Error('Incompatible event protocol'));
          else onEvent(message.envelope);

        else onDisconnect(message.error);
      };

      try {
        cleanups.push(await transport.listen<Message>('daemon-event', (event) => deliver(event.payload)));
        cleanups.push(await transport.listen<Message>('daemon-disconnected', (event) => deliver(event.payload)));
        id = await transport.invoke<number>('daemon_subscribe', { afterSequence });
        for (const message of queued) deliver(message);
      }
      catch (error) { active = false; for (const cleanup of cleanups) cleanup(); throw error }

      return async () => {
        active = false;
        for (const cleanup of cleanups) cleanup();
        await transport.invoke('daemon_unsubscribe', { subscriptionId: id });
      };
    },
  };
}

export async function listenDesktopErrors(onError: (error: unknown) => void): Promise<() => void> {
  const unlisten = await listen('desktop-error', (event) => onError(event.payload));

  try {
    const error = await invoke('desktop_error');

    if (error) onError(error);
  }
  catch (error) { unlisten(); throw error }

  return unlisten;
}
