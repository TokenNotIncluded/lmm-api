#!/usr/bin/env node

import {
  TaxCategory,
  WaffoPancake,
  WaffoPancakeError,
  WebhookEventType,
} from '@waffo/pancake-ts'

const DEFAULT_PRODUCT_NAME = 'LMM Forge test checkout'
const DEFAULT_STORE_NAME = 'LMM Forge test store'
const DEFAULT_AMOUNT = '1.00'
const TEST_CARD = '4576750000000110'

function fail(message) {
  throw new Error(message)
}

function usage() {
  console.log(`Usage: bun run waffo:pancake:smoke -- [options]

Creates a Waffo Pancake test checkout using the server-side SDK. The Waffo
API key must be a Test key; the SDK binds the environment to the key.

Required environment:
  WAFFO_MERCHANT_ID        Merchant ID from Dashboard → API & Development
  WAFFO_PRIVATE_KEY        RSA private key downloaded with the Test API key

Optional environment:
  WAFFO_PANCAKE_ENV        Must be "test" (default: test)
  WAFFO_PANCAKE_STORE_ID   Existing test store ID
  WAFFO_PANCAKE_PRODUCT_ID  Existing active test product ID
  WAFFO_PANCAKE_RETURN_URL Successful checkout return URL

Options:
  --store-id ID             Select an existing store
  --product-id ID           Select an existing active one-time product
  --amount USD              Test price snapshot (default: ${DEFAULT_AMOUNT})
  --buyer-email EMAIL       Pre-fill buyer email
  --buyer-id ID             Stable merchant-side buyer identity
  --order-id ID             Business order reference
  --return-url URL          Override the checkout return URL
  --webhook-url URL         Register this HTTP endpoint for test webhooks
  --configure-webhook       Add --webhook-url to the selected store
  --help                    Show this help

The script never prints WAFFO_PRIVATE_KEY or the short-lived checkout token.
After opening the checkout URL, use test card ${TEST_CARD} with any future
expiry and three-digit CVC. Project endpoint:
  https://api.lmm.best/api/waffo-pancake/webhook/test
`)
}

function parseArgs(argv) {
  const args = {}
  const valueFlags = new Set([
    '--store-id',
    '--product-id',
    '--amount',
    '--buyer-email',
    '--buyer-id',
    '--order-id',
    '--return-url',
    '--webhook-url',
  ])
  const booleanFlags = new Set(['--configure-webhook', '--help'])
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i]
    if (!valueFlags.has(flag) && !booleanFlags.has(flag)) fail(`unknown option: ${flag}`)
    const key = flag.slice(2).replaceAll('-', '_')
    if (booleanFlags.has(flag)) {
      args[key] = true
      continue
    }
    const value = argv[i + 1]
    if (!value || value.startsWith('--')) fail(`${flag} requires a value`)
    args[key] = value
    i += 1
  }
  return args
}

function requiredEnv(name) {
  const value = process.env[name]?.trim()
  if (!value) fail(`${name} is required; do not paste the private key into source control`)
  return value
}

function summarizeError(error) {
  if (error instanceof WaffoPancakeError) {
    const notices = [...(error.errors ?? []), ...(error.warnings ?? [])]
      .map((notice) => notice.message)
      .filter(Boolean)
    return notices.length > 0 ? `${error.message}: ${notices.join('; ')}` : error.message
  }
  return error instanceof Error ? error.message : String(error)
}

async function queryStores(client) {
  const result = await client.graphql.query({
    query: `query {
      stores(limit: 100) {
        id
        name
        status
        onetimeProducts { id name status }
      }
    }`,
  })
  if (result.errors?.length) {
    fail(`unable to list Waffo stores: ${result.errors.map((e) => e.message).join('; ')}`)
  }
  return result.data?.stores ?? []
}

async function ensureTestCatalog(client, args) {
  const requestedStore = args.store_id || process.env.WAFFO_PANCAKE_STORE_ID?.trim()
  const requestedProduct = args.product_id || process.env.WAFFO_PANCAKE_PRODUCT_ID?.trim()
  const stores = await queryStores(client)

  if (requestedProduct) {
    const store = requestedStore
      ? stores.find((item) => item.id === requestedStore)
      : stores.find((item) => item.onetimeProducts?.some((product) => product.id === requestedProduct))
    if (!store && requestedStore) fail(`store ${requestedStore} was not found for the Test key`)
    return { storeId: store?.id || requestedStore || '', productId: requestedProduct }
  }

  let store
  if (requestedStore) {
    store = stores.find((item) => item.id === requestedStore)
    if (!store) fail(`store ${requestedStore} was not found for the Test key`)
  } else if (stores.length === 0) {
    const created = await client.stores.create({ name: DEFAULT_STORE_NAME })
    store = created.store
    console.log(`Created Test store: ${store.id}`)
  } else if (stores.length === 1) {
    store = stores[0]
  } else {
    fail(`merchant has ${stores.length} stores; pass --store-id instead of guessing`)
  }

  const activeProducts = (store.onetimeProducts ?? []).filter(
    (product) => product.status?.toLowerCase() === 'active'
  )
  if (activeProducts.length === 1) return { storeId: store.id, productId: activeProducts[0].id }
  if (activeProducts.length > 1) {
    fail(`store ${store.id} has multiple active products; pass --product-id instead of guessing`)
  }

  const created = await client.onetimeProducts.create({
    storeId: store.id,
    name: DEFAULT_PRODUCT_NAME,
    description: 'Temporary Test checkout product for LMM Forge',
    prices: { USD: { amount: DEFAULT_AMOUNT, taxCategory: TaxCategory.SaaS } },
    successUrl: args.return_url || process.env.WAFFO_PANCAKE_RETURN_URL?.trim() || undefined,
  })
  console.log(`Created active Test product: ${created.product.id}`)
  return { storeId: store.id, productId: created.product.id }
}

async function configureWebhook(client, storeId, webhookUrl) {
  if (!storeId) fail('--configure-webhook requires a resolvable store ID')
  if (!webhookUrl) fail('--configure-webhook requires --webhook-url')
  const result = await client.webhooks.add({
    storeId,
    channel: 'http',
    url: webhookUrl,
    events: [WebhookEventType.OrderCompleted],
    testMode: true,
  })
  console.log(`Registered Test webhook ${result.webhook.id} for ${webhookUrl}`)
}

async function main() {
  const args = parseArgs(process.argv.slice(2))
  if (args.help) {
    usage()
    return
  }
  const environment = (process.env.WAFFO_PANCAKE_ENV || 'test').trim().toLowerCase()
  if (environment !== 'test') fail('this smoke runner is test-only; use a Test API key and WAFFO_PANCAKE_ENV=test')

  const client = new WaffoPancake({
    merchantId: requiredEnv('WAFFO_MERCHANT_ID'),
    privateKey: requiredEnv('WAFFO_PRIVATE_KEY'),
  })
  const catalog = await ensureTestCatalog(client, args)
  if (args.configure_webhook) await configureWebhook(client, catalog.storeId, args.webhook_url)

  const orderId = args.order_id || `lmm-pancake-test-${Date.now()}`
  const buyerIdentity = args.buyer_id || `lmm-pancake-smoke-${Date.now()}`
  const checkout = await client.checkout.authenticated.create({
    productId: catalog.productId,
    currency: 'USD',
    priceSnapshot: {
      amount: args.amount || DEFAULT_AMOUNT,
      taxCategory: TaxCategory.SaaS,
    },
    buyerIdentity,
    buyerEmail: args.buyer_email || 'checkout-test@example.invalid',
    orderMerchantExternalId: orderId,
    successUrl: args.return_url || process.env.WAFFO_PANCAKE_RETURN_URL?.trim() || undefined,
    darkMode: true,
    expiresInSeconds: 45 * 60,
  })

  console.log(JSON.stringify({
    environment: 'test',
    store_id: catalog.storeId,
    product_id: catalog.productId,
    order_id: orderId,
    session_id: checkout.sessionId,
    expires_at: checkout.expiresAt,
    checkout_url: checkout.checkoutUrl,
    test_card: TEST_CARD,
    webhook_endpoint: args.webhook_url || 'https://api.lmm.best/api/waffo-pancake/webhook/test',
    next: 'Complete the hosted checkout, then confirm order.completed reached the webhook and the local order is settled.',
  }, null, 2))
}

main().catch((error) => {
  console.error(`Waffo Pancake smoke failed: ${summarizeError(error)}`)
  process.exitCode = 1
})
