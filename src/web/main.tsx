import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { invoke, isTauri } from '@tauri-apps/api/core';
import { NativeApp } from './app/NativeApp';
import { RuntimeStore } from './features/runtime/runtime-store';
import { createNativeClient, listenDesktopErrors } from './lib/native-client';

import { App } from './app/App';
import './styles/app.css';

const nativeStore = isTauri() ? new RuntimeStore(createNativeClient()) : null;

createRoot(document.getElementById('root')!).render(<StrictMode>{nativeStore ? <NativeApp onError={listenDesktopErrors} store={nativeStore} onStart={() => invoke('daemon_start')} onStop={() => invoke('daemon_stop')} onQuit={() => invoke('desktop_quit')} /> : <App />}</StrictMode>);
