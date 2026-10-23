import type { Catalog, Preview, Price } from './contract'
export const expected = {
  routing: '1',
  pricing: '1',
  snapshot_sha256: 'a'.repeat(64),
}
export const price: Price = {
  model: 'model-a',
  group: 'default',
  speed: 'standard',
  rule_id: '1',
  billing_mode: 'tokens',
  group_multiplier_ppm: 1000000,
  speed_multiplier_ppm: 1000000,
  currency: 'USD',
  prices: [
    {
      name: 'input',
      unit: 'million_tokens',
      up_to_input_tokens: null,
      amount: { numerator: '7', denominator: '1' },
    },
    {
      name: 'cache_read',
      unit: 'million_tokens',
      up_to_input_tokens: null,
      amount: { numerator: '0', denominator: '1' },
    },
  ],
}
export const catalog: Catalog = {
  contract: 'core.routing-admin.v1',
  expected,
  prices: [price],
  routes: [
    {
      route_id: '1',
      model: 'model-a',
      group: 'default',
      targets: [
        {
          target_id: '1',
          upstream_id: '1',
          model: 'model-a',
          priority: 0,
          weight: 1,
          enabled: true,
        },
        {
          target_id: '2',
          upstream_id: '2',
          model: 'model-a',
          priority: 10,
          weight: 3,
          enabled: true,
        },
      ],
    },
  ],
  exchange_rate: { key: 'USDExchangeRate', revision: '1', raw: '7' },
  exchange_rate_available: true,
  can_edit_prices: true,
  can_edit_priorities: true,
}
export const preview: Extract<Preview, { kind: 'prices' }> = {
  kind: 'prices',
  preview_id: 'b'.repeat(64),
  confirmation_sha256: 'c'.repeat(64),
  expires_at: '253402300799',
  expected,
  next: { routing: '2', pricing: '2', snapshot_sha256: 'd'.repeat(64) },
  source_sha256: 'e'.repeat(64),
  exchange_rate: catalog.exchange_rate,
  changes: [
    {
      model: 'model-a',
      group: 'default',
      speed: 'standard',
      rule_id: '1',
      adjustment: 'divide',
      before: price,
      source: price,
      after: {
        ...price,
        prices: price.prices.map((p) => ({
          ...p,
          amount: {
            numerator: p.name === 'input' ? '1' : '0',
            denominator: '1',
          },
        })),
      },
    },
  ],
}
