import { expect, type Page, type Route, test } from '@playwright/test';

import type { DomainMapping, Settings } from '../../src/shared/types';

const json = (route: Route, body: unknown, status = 200) => route.fulfill({ body: JSON.stringify(body), contentType: 'application/json', status });

async function mockApi(page: Page) {
  let settings: Settings = { schemaVersion: 1, revision: 0, lanAddress: '', upstreamDns: ['1.1.1.1'], mappings: [] };

  await page.route('**/api/**', (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname;

    if ('/api/session' === path) return json(route, { csrfToken: 'test-token' });
    if ('/api/status' === path) return json(route, { lanAddress: settings.lanAddress, desiredRevision: settings.revision, appliedRevision: settings.revision, applyStatus: 'applied', diagnostics: [] });
    if ('/api/settings/network' === path) return json(route, settings);
    if ('/api/domains' === path && 'GET' === request.method()) return json(route, settings);

    const body = request.postDataJSON() as { mapping?: Omit<DomainMapping, 'id'>; patch?: Partial<DomainMapping> };

    if ('/api/domains' === path && 'POST' === request.method() && body.mapping) {
      settings = { ...settings, revision: settings.revision + 1, mappings: [...settings.mappings, { ...body.mapping, id: String(settings.revision + 1) }] };
    }
    else if (path.startsWith('/api/domains/')) {
      const id = decodeURIComponent(path.slice('/api/domains/'.length));

      settings = {
        ...settings,
        revision: settings.revision + 1,
        mappings: 'DELETE' === request.method() ? settings.mappings.filter((mapping) => mapping.id !== id) : settings.mappings.map((mapping) => mapping.id === id ? { ...mapping, ...body.patch } : mapping),
      };
    }
    else {
      return json(route, { error: 'Not found' }, 404);
    }

    return json(route, { settings, apply: { desiredRevision: settings.revision, appliedRevision: settings.revision, status: 'applied' } });
  });
}

test('registers and manages parent and nested domains', async ({ page }) => {
  await mockApi(page);
  await page.goto('/');

  await page.getByRole('textbox', { name: 'Domain', exact: true }).fill('shop.own');
  await page.getByRole('textbox', { name: 'Local port', exact: true }).fill('3000');
  await page.getByRole('button', { name: 'Register domain' }).click();
  await expect(page.getByRole('link', { name: 'shop.own' })).toBeVisible();

  await page.getByRole('textbox', { name: 'Domain', exact: true }).fill('api.shop.own');
  await page.getByRole('textbox', { name: 'Local port', exact: true }).fill('8080');
  await page.getByRole('button', { name: 'Register domain' }).click();
  const nestedRow = page.getByRole('listitem').filter({ hasText: 'api.shop.own' });

  await nestedRow.getByRole('button', { name: 'Turn TLS on' }).click();
  await expect(nestedRow).toContainText('HTTPS');
  await nestedRow.getByRole('button', { name: 'Edit api.shop.own' }).click();
  await nestedRow.getByLabel('Upstream port for api.shop.own').fill('4173');
  await nestedRow.getByRole('button', { name: 'Save api.shop.own' }).click();
  await expect(nestedRow).toContainText('127.0.0.1:4173');
  await nestedRow.getByRole('button', { name: 'Disable' }).click();
  await expect(nestedRow.getByRole('button', { name: 'Enable' })).toBeVisible();

  page.on('dialog', (dialog) => dialog.accept());
  await nestedRow.getByRole('button', { name: 'Delete' }).click();
  await expect(page.getByRole('link', { name: 'api.shop.own' })).toHaveCount(0);
});
