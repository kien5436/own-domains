import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { invoke } from '@tauri-apps/api/core';
import { NativeApp } from './app/NativeApp';
import { RuntimeStore } from './features/runtime/runtime-store';
import { createNativeClient, listenDesktopErrors } from './lib/native-client';

import './styles/app.css';

// Single shell: the legacy browser/HTTP app is retired. Outside Tauri the
// store simply never connects and the shell renders its disconnected state.
const store = new RuntimeStore(createNativeClient());

createRoot(document.getElementById('root')!).render(<StrictMode><NativeApp onError={listenDesktopErrors} store={store} onStart={() => invoke('daemon_start')} onStop={() => invoke('daemon_stop')} onQuit={() => invoke('desktop_quit')} /></StrictMode>);
