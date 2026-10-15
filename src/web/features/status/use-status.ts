import { useEffect, useState } from 'react';

import { loadStatus, type StatusResponse } from './api';

const emptyStatus: StatusResponse = { lanAddress: '', desiredRevision: 0, appliedRevision: 0, applyStatus: 'pending', diagnostics: [] };

export function useStatus() {
  const [status, setStatus] = useState(emptyStatus);

  useEffect(() => {
    void Promise.resolve().then(loadStatus).then(setStatus, () => setStatus(emptyStatus));
  }, []);

  return status;
}
